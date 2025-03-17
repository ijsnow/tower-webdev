mod hyper_reverse_proxy;

use std::{
    task::{Context, Poll},
    time::Duration,
};

use futures_util::future::BoxFuture;
use http::{Request, Response, StatusCode};
use http_body::Body as HttpBody;
use http_body_util::Either;
use hyper::body::Incoming;
use hyper_reverse_proxy::HyperReverseProxy;
use hyper_util::{
    client::legacy::{
        connect::{Connect, HttpConnector},
        Client,
    },
    rt::TokioExecutor,
};
use tower::Service;

use hyper_reverse_proxy::ProxyError;

pub struct InsecureReverseProxyService<C, Body> {
    pub target: String,
    pub proxy: HyperReverseProxy<C, Body>,
}

pub type HttpReverseProxyService<Body> = InsecureReverseProxyService<HttpConnector, Body>;

impl<C, B> InsecureReverseProxyService<C, B> {
    pub fn new(
        target: impl Into<String>,
        client: Client<C, B>,
    ) -> InsecureReverseProxyService<C, B> {
        Self {
            target: target.into(),
            proxy: HyperReverseProxy::new(client),
        }
    }
}

impl<B> InsecureReverseProxyService<HttpConnector, B> {
    pub fn new_http(target: impl Into<String>) -> InsecureReverseProxyService<HttpConnector, B>
    where
        B: HttpBody + Send,
        B::Data: Send,
    {
        Self {
            target: target.into(),
            proxy: HyperReverseProxy::new(
                Client::builder(TokioExecutor::new())
                    .pool_idle_timeout(Duration::from_secs(30))
                    .build_http(),
            ),
        }
    }
}

impl<C: Clone, B> Clone for InsecureReverseProxyService<C, B> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            target: self.target.clone(),
            proxy: self.proxy.clone(),
        }
    }
}

pub type InsecureReverseProxyServiceBody = Either<Incoming, String>;

impl<C, Body> Service<Request<Body>> for InsecureReverseProxyService<C, Body>
where
    C: Connect + Clone + Send + Sync + 'static,
    Body: HttpBody + Send + 'static + Unpin,
    Body::Data: Send,
    Body::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = Response<InsecureReverseProxyServiceBody>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let target = self.target.clone();
        let proxy = self.proxy.clone();

        Box::pin(async move {
            let res = proxy
                .call("127.0.0.1".parse().unwrap(), target.clone(), request)
                .await;

            let res = match res {
                Ok(res) => res.map(Either::Left),
                Err(err) => match err {
                    ProxyError::HyperClientError(error) if error.is_connect() => {
                        Response::builder()
                            .status(StatusCode::BAD_GATEWAY)
                            .body(Either::Right(
                                "Bad gateway. Is your dev server running?".to_owned(),
                            ))
                            .unwrap()
                    }
                    error => Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Either::Right(error.to_string()))
                        .unwrap(),
                },
            };

            Ok(res)
        })
    }
}
