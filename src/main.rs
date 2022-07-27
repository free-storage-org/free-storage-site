use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderMap, Request},
    response::Html,
    routing::{get, post},
    Extension, Json, Router,
};
use free_storage::FileId;
use futures_util::stream::StreamExt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    repo: String,
    token: String,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let port = std::env::var("PORT")
        .unwrap_or_else(|_e| "8080".to_owned())
        .parse::<u16>()
        .expect("PORT must be a valid port");
    let repo = std::env::var("GITHUB_REPO").expect("GITHUB_REPO must be set");
    if repo.split('/').count() != 2 {
        panic!("GITHUB_REPO must be in the format owner/repo");
    }
    let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN must be set");

    let state = State { repo, token };

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    println!("listening on http://localhost:{port}/");

    axum::Server::bind(&addr)
        .serve(
            Router::new()
                .route("/", get(|| async { Html(include_str!("index.html")) }))
                .route("/upload", post(upload))
                .route("/get", get(get_file))
                .layer(Extension(state))
                .into_make_service(),
        )
        .await
        .unwrap();
}

async fn upload(
    Extension(State { token, repo }): Extension<State>,
    mut req: Request<Body>,
) -> Json<FileId> {
    let name = req
        .headers()
        .get("X-File-Name")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    println!("uploading {name}");

    let mut bytes = Vec::new();
    while let Some(Ok(part)) = req.body_mut().next().await {
        bytes.extend(part);
    }
    println!("got data for {name}");

    Json(FileId::upload_file(name, bytes, repo, token).await.unwrap())
}
async fn get_file(
    Extension(State { token, .. }): Extension<State>,
    Query(file_id): Query<FileId>,
) -> (HeaderMap, Vec<u8>) {
    let (file_data, file_name) = file_id.get_file(Some(token)).await.unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename={}", file_name)).unwrap(),
    );

    (headers, file_data)
}
