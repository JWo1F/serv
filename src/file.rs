use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use hyper::body::Incoming;
use hyper::header::{
  ACCEPT_ENCODING, ACCEPT_RANGES, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE,
  CONTENT_TYPE, ETAG, IF_NONE_MATCH, IF_RANGE, LAST_MODIFIED, RANGE, VARY,
};
use hyper::{Method, Request, Response, StatusCode};
use mime_guess::Mime;
use mime_guess::mime;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::body::{self, Body};
use crate::compress::{self, Encoding};

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

  let content_type = content_type(path);
  let compressible = compress::worthwhile(&content_type);
  let encoding = if status == StatusCode::OK
    && compressible
    && (compress::MIN_BODY..=compress::MAX_BODY).contains(&len)
  {
    compress::negotiate(header(req, ACCEPT_ENCODING))
  } else {
    None
  };

  let etag = etag(len, modified, encoding);

  let base = || {
    let mut builder = Response::builder()
      .header(CONTENT_TYPE, content_type.clone())
      .header(ETAG, etag.clone())
      // Always ask; never assume. A dev server that told a browser to hold on
      // to a file for an hour would be unusable.
      .header(CACHE_CONTROL, "no-cache");
    if compressible {
      builder = builder.header(VARY, "accept-encoding");
    }
    if let Some(modified) = modified {
      builder = builder.header(LAST_MODIFIED, httpdate::fmt_http_date(modified));
    }
    builder
  };

  if status == StatusCode::OK && matches(header(req, IF_NONE_MATCH), &etag) {
    return Ok(
      base()
        .status(StatusCode::NOT_MODIFIED)
        .body(body::empty())
        .expect("valid response"),
    );
  }

  if let Some(encoding) = encoding {
    return compressed(file, len, encoding, req, base().status(status)).await;
  }

  // `If-Range` lets a resumed download check that the file has not changed
  // underneath it; when it has, the right answer is the whole file.
  let ranged = status == StatusCode::OK
    && header(req, IF_RANGE).is_none_or(|value| matches(Some(value), &etag));

  let range = match header(req, RANGE) {
    Some(spec) if ranged => parse_range(spec, len),
    _ => Range::Whole,
  };

  let (status, start, span) = match range {
    Range::Whole => (status, 0, len),
    Range::Partial(start, end) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
    Range::Unsatisfiable => {
      return Ok(
        base()
          .status(StatusCode::RANGE_NOT_SATISFIABLE)
          .header(ACCEPT_RANGES, "bytes")
          .header(CONTENT_RANGE, format!("bytes */{len}"))
          .header(CONTENT_LENGTH, 0)
          .body(body::empty())
          .expect("valid response"),
      );
    }
  };

  let mut builder = base()
    .status(status)
    .header(ACCEPT_RANGES, "bytes")
    .header(CONTENT_LENGTH, span);
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

/// Buffer the file, compress it off the runtime's threads, and send it whole.
async fn compressed(
  mut file: File,
  len: u64,
  encoding: Encoding,
  req: &Request<Incoming>,
  builder: hyper::http::response::Builder,
) -> io::Result<Response<Body>> {
  let mut data = Vec::with_capacity(len as usize);
  file.read_to_end(&mut data).await?;

  let data = tokio::task::spawn_blocking(move || compress::encode(&data, encoding))
    .await
    .map_err(io::Error::other)??;

  let builder = builder
    .header(CONTENT_ENCODING, encoding.name())
    .header(CONTENT_LENGTH, data.len());

  let content = if req.method() == Method::HEAD {
    body::empty()
  } else {
    body::full(data)
  };

  Ok(builder.body(content).expect("valid response"))
}

/// A validator built from what a `stat` already told us — no hashing, no I/O.
/// The encoding is part of it: a gzipped body is a different representation.
fn etag(len: u64, modified: Option<SystemTime>, encoding: Option<Encoding>) -> String {
  let stamp = modified
    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
    .map(|since| since.as_nanos())
    .unwrap_or(0);
  let suffix = encoding.map(Encoding::tag).unwrap_or_default();
  format!("\"{len:x}-{stamp:x}{suffix}\"")
}

fn header<'a>(req: &'a Request<Incoming>, name: hyper::header::HeaderName) -> Option<&'a str> {
  req
    .headers()
    .get(name)
    .and_then(|value| value.to_str().ok())
}

fn matches(value: Option<&str>, etag: &str) -> bool {
  let Some(value) = value else { return false };
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
