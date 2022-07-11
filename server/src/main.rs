use axum::{routing::get, Router};

mod frontend;
use frontend::frontend;

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .unwrap_or_else(|_e| String::from("8080"))
        .parse::<u16>()
        .expect("PORT must be a valid port");

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    println!("listening on http://localhost:{port}/");

    axum::Server::bind(&addr)
        .serve(Router::new().fallback(get(frontend)).into_make_service())
        .await
        .unwrap();
}
