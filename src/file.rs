use std::io;
use std::path::Path;

use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Method, Response, StatusCode};
use mime_guess::Mime;
use mime_guess::mime;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::body::{self, Body};

/// Send a file from disk, streaming it rather than buffering it.
pub async fn send(path: &Path, method: &Method, status: StatusCode) -> io::Result<Response<Body>> {
    let file = File::open(path).await?;
    let len = file.metadata().await?.len();

    let body = if method == Method::HEAD {
        body::empty()
    } else {
        body::file(file.take(len))
    };

    Ok(Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type(path))
        .header(CONTENT_LENGTH, len)
        .body(body)
        .expect("valid response"))
}

/// Guess a content type, adding a charset for the formats browsers expect one on.
pub fn content_type(path: &Path) -> String {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    if needs_charset(&mime) {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    }
}

fn needs_charset(mime: &Mime) -> bool {
    mime.type_() == mime::TEXT
        || matches!(
            mime.essence_str(),
            "application/javascript"
                | "application/json"
                | "application/xml"
                | "application/manifest+json"
                | "image/svg+xml"
        )
}
