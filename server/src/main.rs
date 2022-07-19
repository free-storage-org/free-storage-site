use axum::{
    extract::Multipart,
    routing::{get, post},
    Extension, Json, Router,
};
use free_storage::FileId;

mod frontend;
use frontend::frontend;

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
                .route("/upload", post(upload))
                .fallback(get(frontend))
                .layer(Extension(state))
                .into_make_service(),
        )
        .await
        .unwrap();
}

async fn upload(
    Extension(State { token, repo }): Extension<State>,
    mut multipart: Multipart,
) -> Json<Vec<FileId>> {
    let mut file_ids = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = String::from(field.file_name().unwrap());
        println!("uploading {file_name}");

        let file_data = field.bytes().await.unwrap();

        // std::fs::write(file_name, file_data);

        let fid = FileId::upload_file(file_name, file_data, repo.clone(), token.clone())
            .await
            .unwrap();
        file_ids.push(fid);
    }

    Json(file_ids)
}
