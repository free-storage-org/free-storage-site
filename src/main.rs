use axum::{
    extract::{
        ws::{Message, WebSocket},
        FromRequest, RequestParts, WebSocketUpgrade,
    },
    http::{header, HeaderMap, Request, StatusCode},
    middleware::{from_fn, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Extension, Router,
};
use axum_extra::extract::{cookie::Cookie, CookieJar};
use free_storage::FileId;
use futures::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use reqwest::header::ACCEPT;
use std::time::Duration;
use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*, EnvFilter};

/// The maximum chunks allowed to be uploaded at once.
///
/// To get the size of the chunks in bytes, multiply this number by 100,000,000.
const MAX_UPLOADED_CHUNKS: u16 = 150;
/// How long to wait for a chunk to be gotten from the request.
///
/// The default is 20 minutes.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// The OAuth client ID to use to sign in with GitHub.
///
/// This should defined in the `.env` file.
static OAUTH_CLIENT_ID: Lazy<String> = Lazy::new(|| match std::env::var("OAUTH_CLIENT_ID") {
    Ok(id) => id,
    Err(_) => {
        tracing::error!("OAUTH_CLIENT_ID is not defined in the environment");
        std::process::exit(1);
    }
});
/// The OAuth client secret to use to sign in with GitHub.
///
/// This should defined in the `.env` file.
static OAUTH_CLIENT_SECRET: Lazy<String> =
    Lazy::new(|| match std::env::var("OAUTH_CLIENT_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            tracing::error!("OAUTH_CLIENT_SECRET is not defined in the environment");
            std::process::exit(1);
        }
    });

async fn auth<B: Send>(
    req: Request<B>,
    next: Next<B>,
) -> Result<impl IntoResponse, Response<String>> {
    fn err() -> Response<String> {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(header::CONTENT_TYPE, "text/html")
            .body(include_str!("login.html").replace("%OAUTH_CLIENT_ID%", &OAUTH_CLIENT_ID))
            .unwrap()
    }

    // running extractors requires a `axum::http::request::Parts`
    let mut parts = RequestParts::new(req);

    let auth = parts.extract::<CookieJar>().await.unwrap();
    let token = String::from(auth.get("github_token").ok_or_else(err)?.value());

    #[derive(serde::Deserialize)]
    struct GitHubUser {
        login: String,
    }

    let GitHubUser {
        login: github_username,
    } = reqwest::Client::builder()
        .user_agent("Rust")
        .build()
        .unwrap()
        .get("https://api.github.com/user")
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(e.to_string())
                .unwrap()
        })?
        .json::<GitHubUser>()
        .await
        .map_err(|_| err())?;
    let repo = format!("{github_username}/__storage");

    // reconstruct the request
    let mut req = parts.try_into_request().unwrap();

    req.extensions_mut().insert(State { token, repo });

    Ok(next.run(req).await)
}

#[derive(Clone, Debug, Default)]
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

    Lazy::force(&OAUTH_CLIENT_ID);
    Lazy::force(&OAUTH_CLIENT_SECRET);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_e| "8080".to_owned())
        .parse::<u16>()
        .expect("PORT must be a valid port");

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on http://localhost:{port}/");

    axum::Server::bind(&addr)
        .serve(
            Router::new()
                .route("/", get(|| async { Html(include_str!("index.html")) }))
                .route("/upload", get(upload))
                .route("/get", get(get_file))
                .layer(from_fn(auth))
                .route("/_/auth", get(authenticate))
                .into_make_service(),
        )
        .await
        .unwrap();
}

#[derive(serde::Deserialize)]
struct GitHubAuth {
    code: String,
}
async fn authenticate(
    jar: CookieJar,
    Qs(GitHubAuth { code }): Qs<GitHubAuth>,
) -> impl IntoResponse {
    #[derive(serde::Deserialize)]
    struct GitHubToken {
        access_token: String,
    }
    let GitHubToken { access_token } = reqwest::Client::new()
        .post(format!(
            "https://github.com/login/oauth/access_token?client_id={}&client_secret={}&code={}",
            &*OAUTH_CLIENT_ID, &*OAUTH_CLIENT_SECRET, code
        ))
        .header(ACCEPT, "application/json")
        .send()
        .await
        .unwrap()
        .json::<GitHubToken>()
        .await
        .unwrap();

    (
        jar.add(
            Cookie::build("github_token", access_token)
                .http_only(true)
                .path("/")
                .finish(),
        ),
        Redirect::to("/"),
    )
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
