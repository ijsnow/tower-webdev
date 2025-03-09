use std::net::SocketAddr;

use axum::Router;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use tower_webdev::{Config, Mode, WebdevService};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tokio::join!(serve(using_serve_dir_with_assets_fallback().await, 4000),);
}

async fn using_serve_dir_with_assets_fallback() -> Router {
    let serve_webdev = WebdevService::new(Config::new_pnpm(Mode::assumed(), "examples/basic-pnpm"))
        .await
        .unwrap();

    Router::new().fallback_service(serve_webdev)
}

async fn serve(app: Router, port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app.layer(TraceLayer::new_for_http()))
        .await
        .unwrap();
}
