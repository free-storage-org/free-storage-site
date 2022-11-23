use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        FromRequest, RequestParts, WebSocketUpgrade,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, Response},
    routing::get,
    Extension, Router,
};
use free_storage::FileId;
use futures::{SinkExt, StreamExt};
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, EnvFilter};

const MAX_UPLOADED_CHUNKS: u16 = 150;
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60); // 20 minutes

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    repo: String,
    token: String,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    tracing_subscriber::registry()
        .with(
            fmt::layer().without_time().compact().with_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            ),
        )
        .init();

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
    tracing::info!("listening on http://localhost:{port}/");

    axum::Server::bind(&addr)
        .serve(
            Router::new()
                .route("/", get(|| async { Html(include_str!("index.html")) }))
                .route("/upload", get(upload))
                .route("/get", get(get_file))
                .layer(Extension(state))
                .into_make_service(),
        )
        .await
        .unwrap();
}

// ~~ripped~~ from https://github.com/tokio-rs/axum/issues/434#issuecomment-954898159
struct Qs<T>(T);

#[axum::async_trait]
impl<T, B: Send> FromRequest<B> for Qs<T>
where
    T: serde::de::DeserializeOwned,
{
    type Rejection = (StatusCode, String);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        fn err(e: impl std::fmt::Display) -> (StatusCode, String) {
            (StatusCode::BAD_REQUEST, e.to_string())
        }

        let query = req.uri().query().ok_or_else(|| err("No GET Parameters"))?;
        Ok(Self(
            serde_qs::Config::new(5, false)
                .deserialize_str(query)
                .map_err(err)?,
        ))
    }
}

async fn get_file(
    Extension(State { token, .. }): Extension<State>,
    Qs(file_id): Qs<FileId>,
) -> (HeaderMap, Vec<u8>) {
    tracing::trace!(?file_id, "getting file");
    let (data, name) = file_id.get(Some(token)).await.unwrap();

    tracing::trace!("got file {name}");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename={}", name)).unwrap(),
    );

    (headers, data)
}

// the whole reason we're using WebSockets is to avoid Railway's 5 minute request timeout

async fn upload(
    Extension(State { token, repo }): Extension<State>,
    ws: WebSocketUpgrade,
) -> Response {
    let handler = |socket: WebSocket| async move {
        let (mut sink, mut stream) = socket.split();

        let file_name = if let Some(msg) = stream.next().await {
            let Ok(msg) = msg else {
                return
            };
            msg.into_text().unwrap()
        } else {
            return;
        };

        let data = match tokio::time::timeout(UPLOAD_TIMEOUT, async {
            let mut data = Vec::new();

            for _ in 0..MAX_UPLOADED_CHUNKS {
                match stream.next().await {
                    Some(Ok(Message::Binary(b))) => {
                        tracing::trace!("got binary message with len: {}", b.len());
                        data.extend(b);
                    }
                    Some(Ok(Message::Text(t))) => {
                        tracing::trace!("got text message with len: {}", t.len());
                        if t == "done" {
                            tracing::trace!("got done message");
                            break;
                        } else {
                            data.extend(t.as_bytes());
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tracing::trace!(?frame, "got close message");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::trace!(?e, "websocket probably closed");
                        break;
                    }
                    None => {
                        tracing::trace!("websocket closed");
                        break;
                    }
                    Some(Ok(ty)) => {
                        tracing::trace!("got other message: {ty:#?}");
                    }
                }
            }
            data
        })
        .await
        {
            Ok(data) => data,
            Err(_) => {
                tracing::trace!("timed out");
                return;
            }
        };

        tracing::trace!("got data with len: {}", data.len());

        let fid = FileId::upload(file_name, &*data, repo, token)
            .await
            .unwrap();

        tracing::trace!(?fid, "uploaded file");

        let _ = sink
            .send(Message::Binary(rmp_serde::to_vec(&fid).unwrap()))
            .await;

        tracing::trace!("sent file id");
    };

    ws.max_send_queue(MAX_UPLOADED_CHUNKS as usize)
        .max_message_size(105_000_000)
        .on_upgrade(handler)
}
