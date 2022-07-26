use axum::{
    extract::{Multipart, Query},
    http::{header, HeaderMap},
    response::Html,
    routing::{get, post},
    Extension, Json, Router,
};
use free_storage::FileId;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    repo: String,
    token: String,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let port = std::env::var("PORT")
        .unwrap_or_else(|_e| String::from("8080"))
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
    mut multipart: Multipart,
) -> Json<Option<FileId>> {
    // only allow one file per request
    // to not take too long processing the request
    if let Ok(Some(field)) = multipart.next_field().await {
        let file_name = String::from(field.file_name().unwrap());
        println!("uploading {file_name}");

        let file_data = field.bytes().await.unwrap();

        let file_id = FileId::upload_file(file_name, file_data, repo.clone(), token.clone())
            .await
            .unwrap();

        Json(Some(file_id))
    } else {
        Json(None)
    }
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
