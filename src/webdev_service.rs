use std::{
    path::PathBuf,
    process::Stdio,
    task::{Context, Poll},
};

use futures_util::future::BoxFuture;
use http::{Request, Response};
use http_body::Body as HttpBody;
use http_body_util::Either;
use hyper::body::Incoming;
use insecure_reverse_proxy::{HttpReverseProxyService, InsecureReverseProxyService};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdout, Command},
};
use tower::Service;
use tower_http::services::{fs::ServeFileSystemResponseBody, ServeDir};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Compile all pages on startup
    mode: Mode,
    /// The command in the $PATH that is assumed to run for web project. e.g. pnpm, npm, yarn, etc.
    command: String,
    /// The subcommand for `self.command` that will install dependencies.
    install_command: String,
    /// Directory to execute the command in.
    root: PathBuf,
    /// Path for the output files
    target: PathBuf,
    /// Dev server port to proxy.
    dev_server_port: u32,
}

impl Config {
    pub fn new_pnpm(mode: Mode, root: impl Into<PathBuf>) -> Self {
        let root = root.into();

        Self {
            mode,
            command: "pnpm".into(),
            install_command: "install".into(),
            target: root.join("dist"),
            root,
            dev_server_port: 3000,
        }
    }

    pub fn root(mut self, value: impl Into<PathBuf>) -> Self {
        self.target = value.into();

        self
    }

    pub fn target(mut self, value: impl Into<PathBuf>) -> Self {
        self.target = value.into();

        self
    }

    pub fn dev_server_port(mut self, value: u32) -> Self {
        self.dev_server_port = value;

        self
    }

    fn ensure_target_exists(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.target)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Production,
    Development,
}

impl Mode {
    pub fn assumed() -> Self {
        #[cfg(debug_assertions)]
        let mode = Self::Development;

        #[cfg(not(debug_assertions))]
        let mode = Self::Production;

        mode
    }
}

pub struct WebdevService<B> {
    config: Config,
    inner_service: InnerService<B>,
}

impl<B> Clone for WebdevService<B> {
    fn clone(&self) -> Self {
        WebdevService {
            config: self.config.clone(),
            inner_service: self.inner_service.clone(),
        }
    }
}

impl<B> WebdevService<B> {
    pub async fn new(
        config: Config,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        B: HttpBody + Send + Unpin + 'static,
        B::Data: Send,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        config.ensure_target_exists()?;

        let this = Self {
            inner_service: InnerService::from_config(&config),
            config,
        };

        this.execute_install().await?;

        match &this.config.mode {
            Mode::Development => {
                this.execute_dev().await?;
            }
            Mode::Production => {
                this.execute_dev().await?;
            }
        }

        Ok(this)
    }
}

pub type WebdevResponse = Either<ServeFileSystemResponseBody, Incoming>;

impl<Body> Service<Request<Body>> for WebdevService<Body>
where
    Body: HttpBody + Send + Unpin + 'static,
    Body::Data: Send,
    Body::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = Response<WebdevResponse>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match &mut self.inner_service {
            InnerService::ServeDir(serve_dir) => {
                <ServeDir as Service<Request<Body>>>::poll_ready(serve_dir, cx)
            }
            InnerService::ReverseProxy(proxy) => {
                <HttpReverseProxyService<Body> as Service<Request<Body>>>::poll_ready(proxy, cx)
            }
        }
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        match &self.inner_service {
            InnerService::ServeDir(serve_dir) => {
                let mut serve_dir = serve_dir.clone();

                Box::pin(async move {
                    let res = serve_dir.call(request).await.unwrap();

                    Ok(res.map(Either::Left))
                })
            }
            InnerService::ReverseProxy(proxy) => {
                let mut proxy = proxy.clone();

                Box::pin(async move {
                    let res = proxy.call(request).await.unwrap();

                    Ok(res.map(Either::Right))
                })
            }
        }
    }
}

enum InnerService<Body> {
    ReverseProxy(HttpReverseProxyService<Body>),
    ServeDir(ServeDir),
}

impl<B> Clone for InnerService<B> {
    fn clone(&self) -> Self {
        match self {
            Self::ReverseProxy(p) => Self::ReverseProxy(p.clone()),
            Self::ServeDir(s) => Self::ServeDir(s.clone()),
        }
    }
}

impl<Body> InnerService<Body> {
    fn from_config(config: &Config) -> Self
    where
        Body: HttpBody + Send + Unpin + 'static,
        Body::Data: Send,
    {
        match &config.mode {
            Mode::Development => Self::ReverseProxy(InsecureReverseProxyService::new_http(
                format!("http://localhost:{}", config.dev_server_port),
            )),
            _ => {
                let serve_dir = ServeDir::new(&config.target);

                Self::ServeDir(serve_dir)
            }
        }
    }
}

#[allow(unused)]
impl<B> WebdevService<B> {
    async fn execute_install(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut command = Command::new(&self.config.command);
        command.current_dir(&self.config.root.canonicalize()?);

        command.args(["install"]);
        command.stdout(Stdio::piped());

        let mut build_process = command.spawn()?;

        let stdout = build_process
            .stdout
            .take()
            .expect("build_process did not have a handle to stdout");

        write_stdout(stdout, "install");

        match build_process.wait().await {
            Ok(status) => {
                if !status.success() {
                    tracing::error!("build process exited with error");

                    std::process::exit(status.code().unwrap_or(1));
                }
            }
            Err(error) => {
                tracing::error!("error waiting for build process: {}", error);
            }
        }

        Ok(())
    }

    async fn execute_build(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut command = Command::new(&self.config.command);
        command.current_dir(&self.config.root.canonicalize()?);

        command.args(["build"]);
        command.stdout(Stdio::piped());

        let mut build_process = command.spawn()?;

        let stdout = build_process
            .stdout
            .take()
            .expect("build_process did not have a handle to stdout");

        write_stdout(stdout, "build");

        match build_process.wait().await {
            Ok(status) => {
                if !status.success() {
                    tracing::error!("build process exited with error");

                    std::process::exit(status.code().unwrap_or(1));
                }
            }
            Err(error) => {
                tracing::error!("error waiting for build process: {}", error);
            }
        }

        Ok(())
    }

    async fn execute_dev(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        let mut command = Command::new(&self.config.command);
        command.current_dir(&self.config.root.canonicalize()?);
        command.args(["dev"]);
        command.stdout(Stdio::piped());

        let mut build_process = command.spawn()?;

        let stdout = build_process
            .stdout
            .take()
            .expect("dev process did not have a handle to stdout");

        write_stdout(stdout, "dev");

        tokio::spawn(async move {
            match build_process.wait().await {
                Ok(status) => {
                    if !status.success() {
                        tracing::error!("dev process exited with error");

                        std::process::exit(status.code().unwrap_or(1));
                    }
                }
                Err(error) => {
                    tracing::error!("error waiting for dev process: {}", error);
                }
            }
        });

        Ok(())
    }
}

fn write_stdout(stdout: ChildStdout, prefix: &'static str) {
    let mut reader = BufReader::new(stdout).lines();

    tokio::spawn(async move {
        let mut output = tokio::io::stdout();

        while let Ok(Some(line)) = reader.next_line().await {
            output
                .write_all(format!("webdev {prefix}: ").as_bytes())
                .await
                .unwrap();
            output.write_all(line.as_bytes()).await.unwrap();
            output.write_all(b"\n").await.unwrap();
            output.flush().await.unwrap();
        }
    });
}
