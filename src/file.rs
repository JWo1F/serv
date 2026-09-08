use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use hyper::body::Incoming;
use hyper::header::{
  ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
  IF_RANGE, LAST_MODIFIED, RANGE,
};
use hyper::{Method, Request, Response, StatusCode};
use mime_guess::Mime;
use mime_guess::mime;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::body::{self, Body};

/// Send a file from disk.
///
/// Nothing is held between requests: the file is opened, measured and streamed
/// every time. The only cache serv takes part in is the browser's, and only
/// through revalidation — an `ETag` built from the file's size and modification
/// time, answered with `304` when it still matches.
pub async fn send(
  path: &Path,
  req: &Request<Incoming>,
  status: StatusCode,
) -> io::Result<Response<Body>> {
  let mut file = File::open(path).await?;
  let meta = file.metadata().await?;
  let len = meta.len();
  let modified = meta.modified().ok();
  let etag = etag(len, modified);

  let base = || {
    let mut builder = Response::builder()
      .header(CONTENT_TYPE, content_type(path))
      .header(ETAG, etag.clone())
      .header(ACCEPT_RANGES, "bytes")
      // Always ask; never assume. A dev server that told a browser to hold on
      // to a file for an hour would be unusable.
      .header(CACHE_CONTROL, "no-cache");
    if let Some(modified) = modified {
      builder = builder.header(LAST_MODIFIED, httpdate::fmt_http_date(modified));
    }
    builder
  };

  if status == StatusCode::OK && matches(req.headers().get(IF_NONE_MATCH), &etag) {
    return Ok(
      base()
        .status(StatusCode::NOT_MODIFIED)
        .body(body::empty())
        .expect("valid response"),
    );
  }

  // `If-Range` lets a resumed download check that the file has not changed
  // underneath it; when it has, the right answer is the whole file.
  let ranged = status == StatusCode::OK
    && req
      .headers()
      .get(IF_RANGE)
      .is_none_or(|value| matches(Some(value), &etag));

  let range = match req.headers().get(RANGE).and_then(|v| v.to_str().ok()) {
    Some(header) if ranged => parse_range(header, len),
    _ => Range::Whole,
  };

  let (status, start, span) = match range {
    Range::Whole => (status, 0, len),
    Range::Partial(start, end) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
    Range::Unsatisfiable => {
      return Ok(
        base()
          .status(StatusCode::RANGE_NOT_SATISFIABLE)
          .header(CONTENT_RANGE, format!("bytes */{len}"))
          .header(CONTENT_LENGTH, 0)
          .body(body::empty())
          .expect("valid response"),
      );
    }
  };

  let mut builder = base().status(status).header(CONTENT_LENGTH, span);
  if status == StatusCode::PARTIAL_CONTENT {
    let end = start + span - 1;
    builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
  }

  let content = if req.method() == Method::HEAD {
    body::empty()
  } else {
    if start > 0 {
      file.seek(io::SeekFrom::Start(start)).await?;
    }
    body::file(file.take(span))
  };

  Ok(builder.body(content).expect("valid response"))
}

/// A validator built from what a `stat` already told us — no hashing, no I/O.
fn etag(len: u64, modified: Option<SystemTime>) -> String {
  let stamp = modified
    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
    .map(|since| since.as_nanos())
    .unwrap_or(0);
  format!("\"{len:x}-{stamp:x}\"")
}

fn matches(header: Option<&hyper::header::HeaderValue>, etag: &str) -> bool {
  let Some(value) = header.and_then(|value| value.to_str().ok()) else {
    return false;
  };
  value == "*" || value.split(',').any(|candidate| candidate.trim() == etag)
}

enum Range {
  Whole,
  /// Inclusive byte offsets, as HTTP counts them.
  Partial(u64, u64),
  Unsatisfiable,
}

/// Parse a `Range` header. Only a single range is honoured; asking for several
/// is answered with the whole file, which is a response every client accepts.
fn parse_range(header: &str, len: u64) -> Range {
  let Some(spec) = header.strip_prefix("bytes=") else {
    return Range::Whole;
  };
  if spec.contains(',') {
    return Range::Whole;
  }
  let Some((from, to)) = spec.trim().split_once('-') else {
    return Range::Whole;
  };

  let (start, end) = match (from.trim(), to.trim()) {
    // `-500`: the last 500 bytes.
    ("", suffix) => match suffix.parse::<u64>() {
      Ok(0) | Err(_) => return Range::Unsatisfiable,
      Ok(count) => (len.saturating_sub(count), len - 1),
    },
    (start, "") => match start.parse::<u64>() {
      Ok(start) => (start, len - 1),
      Err(_) => return Range::Whole,
    },
    (start, end) => match (start.parse::<u64>(), end.parse::<u64>()) {
      (Ok(start), Ok(end)) => (start, end.min(len - 1)),
      _ => return Range::Whole,
    },
  };

  if len == 0 || start > end || start >= len {
    Range::Unsatisfiable
  } else {
    Range::Partial(start, end)
  }
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
