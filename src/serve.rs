use std::convert::Infallible;
use std::io;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, LOCATION};
use hyper::{Method, Request, Response, StatusCode};

use crate::body::{self, Body};
use crate::config::Config;
use crate::{file, path};

pub async fn handle(req: Request<Incoming>, config: Arc<Config>) -> Result<Response<Body>, Infallible> {
    Ok(match route(&req, &config).await {
        Ok(response) => response,
        Err(err) => {
            log::error!("{} {}: {err}", req.method(), req.uri().path());
            text(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        }
    })
}

async fn route(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return Ok(text(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"));
    }

    let url_path = req.uri().path();
    let Some(target) = path::resolve(&config.root, url_path) else {
        return Ok(not_found());
    };
    if !path::is_within(&config.root, &target) {
        return Ok(not_found());
    }

    let Ok(meta) = tokio::fs::metadata(&target).await else {
        return Ok(not_found());
    };

    if meta.is_dir() {
        // Relative links inside the page only resolve correctly under a trailing slash.
        if !url_path.ends_with('/') {
            return Ok(redirect(&format!("{url_path}/")));
        }
        let index = target.join("index.html");
        if tokio::fs::metadata(&index).await.is_ok_and(|m| m.is_file()) {
            return file::send(&index, req.method(), StatusCode::OK).await;
        }
        return Ok(not_found());
    }

    file::send(&target, req.method(), StatusCode::OK).await
}

fn redirect(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(LOCATION, location)
        .body(body::empty())
        .expect("valid response")
}

fn not_found() -> Response<Body> {
    text(StatusCode::NOT_FOUND, "not found")
}

fn text(status: StatusCode, message: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body::full(message))
        .expect("valid response")
}
