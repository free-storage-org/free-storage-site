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
use futures::{stream::SplitStream, SinkExt, StreamExt};
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, EnvFilter};

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

struct WebSocketReader {
    stream: SplitStream<WebSocket>,
    overflow: Vec<u8>,
}

impl WebSocketReader {
    fn new(stream: SplitStream<WebSocket>) -> Self {
        Self {
            stream,
            overflow: Vec::new(),
        }
    }
}

impl std::io::Read for WebSocketReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        tracing::trace!("reading {} bytes from websocket", buf.len());
        tracing::trace!("overflow: {}", self.overflow.len());
        let (remaining, idx) = if self.overflow.len() > buf.len() {
            // We have more data overflowing than the buffer can hold, so we need to split it up

            tracing::trace!("overflow buffer not overflowing buffer");

            buf.copy_from_slice(&self.overflow[..buf.len()]); // Copy the first part of the overflow buffer
            self.overflow.drain(..buf.len()); // Remove the parts we copied
            return Ok(buf.len()); // We can't do anything else, so return the number of bytes we copied
        } else {
            // We have less data overflowing than the buffer can hold, so we can copy it all and then read more

            tracing::trace!("overflow buffer not overflowing buffer");

            buf[..self.overflow.len()].copy_from_slice(&self.overflow); // Copy the overflow buffer
            self.overflow.drain(..); // Clear the overflow buffer
            (buf.len() - self.overflow.len(), self.overflow.len()) // Return the remaining buffer and the index to start writing at
        };

        futures::executor::block_on(async move {
            let mut handle_bytes = |b: &[u8]| {
                if b.len() > remaining {
                    // We have more bytes than we can fit in the buffer, so we
                    // need to store the extra bytes for the next read.

                    tracing::info!("{} bytes overflowing buffer", b.len() - remaining);

                    buf[idx..idx + remaining].copy_from_slice(&b[..remaining]); // Copy the bytes we can fit into the buffer
                    self.overflow.extend_from_slice(&b[remaining..]); // Store the overflow bytes for the next read
                    Ok(idx + remaining) // Return the number of bytes we copied
                } else {
                    tracing::trace!("{} bytes not overflowing buffer", b.len());

                    buf[idx..idx + b.len()].copy_from_slice(b);
                    Ok(idx + b.len())
                }
            };

            tracing::trace!("reading from websocket");

            match self.stream.next().await {
                Some(Ok(Message::Binary(b))) => {
                    tracing::trace!("got binary message with len: {}", b.len());
                    handle_bytes(&b)
                }
                Some(Ok(Message::Text(t))) => {
                    tracing::trace!("got text message with len: {}", t.len());
                    if t == "done" {
                        tracing::trace!("got done message");
                        Ok(0)
                    } else {
                        handle_bytes(t.as_bytes())
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    tracing::trace!(?frame, "got close message");
                    Ok(0)
                }
                Some(Err(e)) => {
                    tracing::trace!(?e, "websocket probably closed");
                    Ok(0)
                }
                None => {
                    tracing::trace!("websocket closed");
                    Ok(0)
                }
                Some(Ok(ty)) => {
                    tracing::trace!("got other message: {ty:#?}");
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "ping/pong",
                    ))
                }
            }
        })
    }
}

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

        fn handle_bytes(b: &[u8]) {
            std::fs::write(format!("test-{}.bin", b.len()), b).unwrap();
        }

        match stream.next().await {
            Some(Ok(Message::Binary(b))) => {
                tracing::trace!("got binary message with len: {}", b.len());
                handle_bytes(&b)
            }
            Some(Ok(Message::Text(t))) => {
                tracing::trace!("got text message with len: {}", t.len());
                if t == "done" {
                    tracing::trace!("got done message");
                } else {
                    handle_bytes(t.as_bytes())
                }
            }
            Some(Ok(Message::Close(frame))) => {
                tracing::trace!(?frame, "got close message");
            }
            Some(Err(e)) => {
                tracing::trace!(?e, "websocket probably closed");
            }
            None => {
                tracing::trace!("websocket closed");
            }
            Some(Ok(ty)) => {
                tracing::trace!("got other message: {ty:#?}");
            }
        }

        // let fid = FileId::upload(file_name, &mut WebSocketReader::new(stream), repo, token)
        //     .await
        //     .unwrap();

        // let _ = sink
        //     .send(Message::Binary(rmp_serde::to_vec(&fid).unwrap()))
        //     .await;
    };

    ws.max_message_size(105_000_000).on_upgrade(handler)
}
