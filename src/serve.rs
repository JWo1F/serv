use std::convert::Infallible;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use damask::Component;
use hyper::body::Incoming;
use hyper::header::{ACCEPT, CONTENT_TYPE, HOST, LOCATION};
use hyper::{Method, Request, Response, StatusCode};

use crate::body::{self, Body};
use crate::config::Config;
use crate::logging;
use crate::pages::{listing, not_found::NotFound};
use crate::{file, path};

pub async fn handle(
  req: Request<Incoming>,
  config: Arc<Config>,
) -> Result<Response<Body>, Infallible> {
  let started = Instant::now();

  let response = match route(&req, &config).await {
    Ok(response) => response,
    Err(err) => {
      log::error!("{} {}: {err}", req.method(), req.uri().path());
      text(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
  };

  let size = response
    .headers()
    .get(hyper::header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse().ok());

  logging::request(
    req.method().as_str(),
    req.uri().path(),
    response.status().as_u16(),
    size,
    started.elapsed(),
  );

  Ok(response)
}

async fn route(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if !matches!(*req.method(), Method::GET | Method::HEAD) {
    return Ok(text(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"));
  }

  let url_path = req.uri().path();
  let Some(target) = path::resolve(&config.root, url_path) else {
    return miss(req, config).await;
  };
  if !path::is_within(&config.root, &target) {
    return miss(req, config).await;
  }

  // Clean URLs: `/about.html` is the same page as `/about`, so send visitors
  // to the canonical one instead of serving both.
  if !config.ext && url_path.ends_with(".html") && is_file(&target).await {
    return Ok(redirect(req, strip_html(url_path)));
  }

  if is_file(&target).await {
    return file::send(&target, req, StatusCode::OK).await;
  }

  if is_dir(&target).await {
    // Relative links inside the page only resolve correctly under a trailing slash.
    if !url_path.ends_with('/') {
      return Ok(redirect(req, format!("{url_path}/")));
    }
    let index = target.join("index.html");
    if is_file(&index).await {
      return file::send(&index, req, StatusCode::OK).await;
    }
    let index = listing::read(&target, url_path, &config.root).await?;
    return Ok(html(StatusCode::OK, req.method(), index.render()));
  }

  if !config.ext {
    let candidate = with_html_suffix(&target);
    if is_file(&candidate).await {
      return file::send(&candidate, req, StatusCode::OK).await;
    }
  }

  miss(req, config).await
}

/// Nothing on disk matched. A single-page app answers with its shell; anything
/// else gets the not-found page.
async fn miss(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if let Some(spa) = &config.spa
    && wants_html(req)
    && is_file(spa).await
  {
    return file::send(spa, req, StatusCode::OK).await;
  }
  not_found(req, config).await
}

/// Only navigations get the SPA shell — a missing script should stay a 404
/// rather than turn into an HTML file with a confusing MIME type. Browsers ask
/// for `text/html` when navigating; a bare path with no extension is treated as
/// a route too, so `curl /some/route` behaves the way you would expect.
fn wants_html(req: &Request<Incoming>) -> bool {
  let accepts_html = req
    .headers()
    .get(ACCEPT)
    .and_then(|value| value.to_str().ok())
    .is_some_and(|accept| accept.contains("text/html"));

  accepts_html || !last_segment(req.uri().path()).contains('.')
}

fn last_segment(url_path: &str) -> &str {
  url_path.rsplit('/').next().unwrap_or("")
}

fn strip_html(url_path: &str) -> String {
  let stem = url_path.trim_end_matches(".html");
  match stem.rsplit_once('/') {
    // `/docs/index.html` is `/docs/`, and `/index.html` is `/`.
    Some((parent, "index")) => format!("{parent}/"),
    _ => stem.to_string(),
  }
}

fn with_html_suffix(path: &std::path::Path) -> PathBuf {
  let mut name = OsString::from(path);
  name.push(".html");
  PathBuf::from(name)
}

async fn is_file(path: &std::path::Path) -> bool {
  tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file())
}

async fn is_dir(path: &std::path::Path) -> bool {
  tokio::fs::metadata(path).await.is_ok_and(|m| m.is_dir())
}

fn redirect(req: &Request<Incoming>, location: String) -> Response<Body> {
  let location = match req.uri().query() {
    Some(query) => format!("{location}?{query}"),
    None => location,
  };
  Response::builder()
    .status(StatusCode::MOVED_PERMANENTLY)
    .header(LOCATION, location)
    .body(body::empty())
    .expect("valid response")
}

/// The not-found page. A `--not-found` file is read on every miss, so editing it
/// shows up on the next reload just like any other file serv hands out.
async fn not_found(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if let Some(page) = &config.not_found
    && is_file(page).await
  {
    return file::send(page, req, StatusCode::NOT_FOUND).await;
  }

  let host = req
    .headers()
    .get(HOST)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("localhost");
  let page = NotFound::new(req.uri().path(), host);

  Ok(html(StatusCode::NOT_FOUND, req.method(), page.render()))
}

fn html(status: StatusCode, method: &Method, markup: String) -> Response<Body> {
  let len = markup.len();
  let body = if method == Method::HEAD {
    body::empty()
  } else {
    body::full(markup)
  };
  Response::builder()
    .status(status)
    .header(CONTENT_TYPE, "text/html; charset=utf-8")
    .header(hyper::header::CONTENT_LENGTH, len)
    .body(body)
    .expect("valid response")
}

fn text(status: StatusCode, message: &'static str) -> Response<Body> {
  Response::builder()
    .status(status)
    .header(CONTENT_TYPE, "text/plain; charset=utf-8")
    .body(body::full(message))
    .expect("valid response")
}
