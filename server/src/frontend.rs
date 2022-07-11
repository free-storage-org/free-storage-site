use axum::http::{header, Uri};

pub async fn frontend(uri: Uri) -> ([(header::HeaderName, &'static str); 1], &'static [u8]) {
    macro_rules! ct {
        ($ct:expr) => {
            [(header::CONTENT_TYPE, $ct)]
        };
    }
    match uri.path() {
        "/assets/client.wasm" => (
            ct!("application/wasm"),
            include_bytes!(concat!(env!("OUT_DIR"), "/client_bg.wasm")),
        ),
        "/assets/client.js" => (
            ct!("text/javascript"),
            include_bytes!(concat!(env!("OUT_DIR"), "/client.js")),
        ),
        _ => (
            ct!("text/html"),
            include_bytes!("../../client/src/index.html"),
        ),
    }
}
