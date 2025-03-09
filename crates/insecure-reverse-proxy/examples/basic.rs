use std::time::Duration;

use axum::Router;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use insecure_reverse_proxy::InsecureReverseProxyService;

#[tokio::main]
async fn main() {
    let client = Client::builder(TokioExecutor::new())
        .pool_idle_timeout(Duration::from_secs(30))
        .build_http();

    let app = Router::new().fallback_service(InsecureReverseProxyService::new(
        "http://localhost:3000",
        client,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:4000")
        .await
        .unwrap();

    println!("listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}
