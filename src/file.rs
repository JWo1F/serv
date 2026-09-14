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

fn header(req: &Request<Incoming>, name: hyper::header::HeaderName) -> Option<&str> {
  req
    .headers()
    .get(name)
    .and_then(|value| value.to_str().ok())
}

fn matches(value: Option<&str>, etag: &str) -> bool {
  let Some(value) = value else { return false };
  value == "*" || value.split(',').any(|candidate| candidate.trim() == etag)
}

#[derive(Debug, PartialEq, Eq)]
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
  // An empty file has no last byte, and every arm below reaches for `len - 1`.
  if len == 0 {
    return Range::Unsatisfiable;
  }
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

  if start > end || start >= len {
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testkit::{Req, Site};

  /// Big enough to be worth compressing, and compressible.
  fn prose() -> String {
    "the quick brown fox jumps over the lazy dog\n".repeat(64)
  }

  // ---- validators and revalidation --------------------------------------

  #[tokio::test]
  async fn every_response_carries_an_etag_and_asks_to_revalidate() {
    let site = Site::new().file("a.txt", "hello");
    let reply = site.get("/a.txt").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("cache-control"), Some("no-cache"));
    let tag = reply.header("etag").expect("an etag");
    assert!(tag.starts_with('"') && tag.ends_with('"'), "{tag}");
    assert!(tag.contains('-'), "{tag}");
  }

  #[tokio::test]
  async fn the_same_file_keeps_the_same_etag() {
    let site = Site::new().file("a.txt", "hello");

    let first = site.get("/a.txt").await;
    let second = site.get("/a.txt").await;
    assert_eq!(first.header("etag"), second.header("etag"));
  }

  #[tokio::test]
  async fn a_changed_file_gets_a_new_etag() {
    let site = Site::new().file("a.txt", "hello");
    let before = site.get("/a.txt").await.header("etag").unwrap().to_string();

    std::fs::write(site.path().join("a.txt"), "hello, again").unwrap();
    let after = site.get("/a.txt").await.header("etag").unwrap().to_string();

    assert_ne!(before, after);
  }

  #[tokio::test]
  async fn a_matching_validator_is_answered_with_304() {
    let site = Site::new().file("a.txt", "hello");
    let tag = site.get("/a.txt").await.header("etag").unwrap().to_string();

    let reply = site
      .send(Req::get("/a.txt").header("if-none-match", &tag))
      .await;

    assert_eq!(reply.status, 304);
    assert!(reply.body.is_empty());
    // The validators come back with the 304, so the browser can keep using them.
    assert_eq!(reply.header("etag"), Some(tag.as_str()));
    assert_eq!(reply.header("cache-control"), Some("no-cache"));
    assert!(reply.has("last-modified"));
  }

  #[tokio::test]
  async fn a_wildcard_validator_matches_anything() {
    let site = Site::new().file("a.txt", "hello");
    let reply = site
      .send(Req::get("/a.txt").header("if-none-match", "*"))
      .await;

    assert_eq!(reply.status, 304);
  }

  #[tokio::test]
  async fn a_validator_in_a_list_still_matches() {
    let site = Site::new().file("a.txt", "hello");
    let tag = site.get("/a.txt").await.header("etag").unwrap().to_string();
    let list = format!("\"stale\", {tag}, \"older\"");

    let reply = site
      .send(Req::get("/a.txt").header("if-none-match", &list))
      .await;

    assert_eq!(reply.status, 304);
  }

  #[tokio::test]
  async fn a_stale_validator_gets_the_file() {
    let site = Site::new().file("a.txt", "hello");
    let reply = site
      .send(Req::get("/a.txt").header("if-none-match", "\"nonsense\""))
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "hello");
  }

  #[tokio::test]
  async fn a_file_reports_when_it_was_last_modified() {
    let site = Site::new().file("a.txt", "hello");
    let reply = site.get("/a.txt").await;

    let stamp = reply.header("last-modified").expect("a date");
    assert!(httpdate::parse_http_date(stamp).is_ok(), "{stamp}");
    assert!(stamp.ends_with("GMT"), "{stamp}");
  }

  #[tokio::test]
  async fn a_not_found_page_is_never_answered_with_304() {
    // Revalidation is only offered for a 200; a 404 body that is up to date is
    // still a 404 the browser has to be told about.
    let site = Site::new().not_found("404.html").file("404.html", "gone");
    let reply = site
      .send(Req::get("/missing").header("if-none-match", "*"))
      .await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.text(), "gone");
  }

  // ---- ranges -----------------------------------------------------------

  #[tokio::test]
  async fn a_whole_file_advertises_that_ranges_are_accepted() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site.get("/a.bin").await;

    assert_eq!(reply.header("accept-ranges"), Some("bytes"));
    assert_eq!(reply.content_length(), Some(10));
    assert!(!reply.has("content-range"));
  }

  #[tokio::test]
  async fn a_closed_range_is_answered_with_206() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::get("/a.bin").header("range", "bytes=2-5"))
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.header("content-range"), Some("bytes 2-5/10"));
    assert_eq!(reply.content_length(), Some(4));
    assert_eq!(reply.text(), "2345");
  }

  #[tokio::test]
  async fn an_open_ended_range_runs_to_the_end() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::get("/a.bin").header("range", "bytes=7-"))
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.header("content-range"), Some("bytes 7-9/10"));
    assert_eq!(reply.text(), "789");
  }

  #[tokio::test]
  async fn a_suffix_range_counts_back_from_the_end() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::get("/a.bin").header("range", "bytes=-3"))
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.header("content-range"), Some("bytes 7-9/10"));
    assert_eq!(reply.text(), "789");
  }

  #[tokio::test]
  async fn a_suffix_longer_than_the_file_is_the_whole_file() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::get("/a.bin").header("range", "bytes=-99"))
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.header("content-range"), Some("bytes 0-9/10"));
    assert_eq!(reply.text(), "0123456789");
  }

  #[tokio::test]
  async fn a_range_starting_past_the_end_is_unsatisfiable() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::get("/a.bin").header("range", "bytes=10-20"))
      .await;

    assert_eq!(reply.status, 416);
    assert_eq!(reply.header("content-range"), Some("bytes */10"));
    assert_eq!(reply.content_length(), Some(0));
    assert!(reply.body.is_empty());
  }

  #[tokio::test]
  async fn a_malformed_range_gets_the_whole_file() {
    // Answering in full is a response every client accepts, which beats failing
    // a download over a header nobody can read.
    let site = Site::new().file("a.bin", "0123456789");

    for spec in [
      "items=0-1",
      "bytes=0-1,4-5",
      "bytes=abc-",
      "bytes",
      "nonsense",
    ] {
      let reply = site.send(Req::get("/a.bin").header("range", spec)).await;
      assert_eq!(reply.status, 200, "{spec}");
      assert_eq!(reply.text(), "0123456789", "{spec}");
    }
  }

  #[tokio::test]
  async fn a_resumed_download_of_an_unchanged_file_gets_its_range() {
    let site = Site::new().file("a.bin", "0123456789");
    let tag = site.get("/a.bin").await.header("etag").unwrap().to_string();

    let reply = site
      .send(
        Req::get("/a.bin")
          .header("if-range", &tag)
          .header("range", "bytes=5-"),
      )
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.text(), "56789");
  }

  #[tokio::test]
  async fn a_resumed_download_of_a_changed_file_gets_the_whole_thing() {
    // The alternative is splicing new bytes onto an old prefix, which is a
    // corrupt file that looks like a successful download.
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(
        Req::get("/a.bin")
          .header("if-range", "\"stale\"")
          .header("range", "bytes=5-"),
      )
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "0123456789");
  }

  #[tokio::test]
  async fn an_if_range_given_as_a_date_falls_back_to_the_whole_file() {
    // Only the entity-tag form is understood. A date never matches the ETag, so
    // the request is treated as a changed file — safe, if conservative.
    let site = Site::new().file("a.bin", "0123456789");
    let stamp = site
      .get("/a.bin")
      .await
      .header("last-modified")
      .unwrap()
      .to_string();

    let reply = site
      .send(
        Req::get("/a.bin")
          .header("if-range", &stamp)
          .header("range", "bytes=5-"),
      )
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "0123456789");
  }

  #[tokio::test]
  async fn head_answers_a_range_with_its_headers_and_no_body() {
    let site = Site::new().file("a.bin", "0123456789");
    let reply = site
      .send(Req::head("/a.bin").header("range", "bytes=2-5"))
      .await;

    assert_eq!(reply.status, 206);
    assert_eq!(reply.header("content-range"), Some("bytes 2-5/10"));
    assert_eq!(reply.content_length(), Some(4));
    assert!(reply.body.is_empty());
  }

  #[tokio::test]
  async fn a_not_found_page_is_never_ranged() {
    // Range handling is gated on a 200; a partial 404 body would be nonsense.
    let site = Site::new()
      .not_found("404.html")
      .file("404.html", "0123456789");
    let reply = site
      .send(Req::get("/missing").header("range", "bytes=2-5"))
      .await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.text(), "0123456789");
  }

  // ---- compression ------------------------------------------------------

  #[tokio::test]
  async fn text_over_a_kilobyte_is_gzipped_when_it_is_welcome() {
    let body = prose();
    let site = Site::new().file("a.txt", &body);
    let reply = site
      .send(Req::get("/a.txt").header("accept-encoding", "gzip, deflate, br"))
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("content-encoding"), Some("gzip"));
    assert_eq!(reply.header("vary"), Some("accept-encoding"));
    assert_eq!(reply.content_length(), Some(reply.body.len() as u64));
    assert!(reply.body.len() < body.len());
    assert_eq!(reply.gunzip(), body.as_bytes());
  }

  #[tokio::test]
  async fn a_compressed_body_gets_an_etag_of_its_own() {
    // A gzipped body is a different representation, and must not be confused
    // with the identity one a previous request cached.
    let site = Site::new().file("a.txt", prose());

    let plain = site.get("/a.txt").await;
    let packed = site
      .send(Req::get("/a.txt").header("accept-encoding", "gzip"))
      .await;

    assert!(packed.header("etag").unwrap().ends_with("-gz\""));
    assert_ne!(plain.header("etag"), packed.header("etag"));
  }

  #[tokio::test]
  async fn a_compressed_body_revalidates_against_its_own_etag() {
    let site = Site::new().file("a.txt", prose());
    let tag = site
      .send(Req::get("/a.txt").header("accept-encoding", "gzip"))
      .await
      .header("etag")
      .unwrap()
      .to_string();

    let matched = site
      .send(
        Req::get("/a.txt")
          .header("accept-encoding", "gzip")
          .header("if-none-match", &tag),
      )
      .await;
    assert_eq!(matched.status, 304);

    // The same tag offered without gzip is the wrong representation.
    let mismatched = site
      .send(Req::get("/a.txt").header("if-none-match", &tag))
      .await;
    assert_eq!(mismatched.status, 200);
  }

  #[tokio::test]
  async fn a_refusal_is_honoured() {
    let site = Site::new().file("a.txt", prose());
    let reply = site
      .send(Req::get("/a.txt").header("accept-encoding", "gzip;q=0"))
      .await;

    assert!(!reply.has("content-encoding"));
    assert_eq!(reply.text(), prose());
  }

  #[tokio::test]
  async fn a_client_that_says_nothing_gets_the_bytes_as_they_are() {
    let site = Site::new().file("a.txt", prose());
    let reply = site.get("/a.txt").await;

    assert!(!reply.has("content-encoding"));
    assert_eq!(reply.text(), prose());
  }

  #[tokio::test]
  async fn anything_under_a_kilobyte_is_left_alone() {
    // Below the threshold the gzip framing costs more than it saves.
    let site = Site::new()
      .file("small.txt", "x".repeat(1023))
      .file("exact.txt", "x".repeat(1024));

    let small = site
      .send(Req::get("/small.txt").header("accept-encoding", "gzip"))
      .await;
    assert!(!small.has("content-encoding"));

    let exact = site
      .send(Req::get("/exact.txt").header("accept-encoding", "gzip"))
      .await;
    assert_eq!(exact.header("content-encoding"), Some("gzip"));
  }

  #[tokio::test]
  async fn anything_over_eight_megabytes_keeps_its_streaming_path() {
    // Past the ceiling the file is never buffered, so it keeps range support
    // and a memory cost that does not track its size.
    let site = Site::new().file("big.txt", "x".repeat(8 * 1024 * 1024 + 1));
    let reply = site
      .send(Req::get("/big.txt").header("accept-encoding", "gzip"))
      .await;

    assert!(!reply.has("content-encoding"));
    assert_eq!(reply.header("accept-ranges"), Some("bytes"));
    assert_eq!(reply.content_length(), Some(8 * 1024 * 1024 + 1));
  }

  #[tokio::test]
  async fn an_already_compressed_format_is_sent_as_it_is() {
    let site = Site::new().file("a.png", "x".repeat(4096));
    let reply = site
      .send(Req::get("/a.png").header("accept-encoding", "gzip"))
      .await;

    assert!(!reply.has("content-encoding"));
    // Nothing varies by encoding, so the response does not claim it does.
    assert!(!reply.has("vary"));
    assert_eq!(reply.header("accept-ranges"), Some("bytes"));
  }

  #[tokio::test]
  async fn a_compressible_type_says_it_varies_even_uncompressed() {
    // A shared cache must not hand a gzipped body to a client that cannot read
    // one, so the header is set whenever the answer could have differed.
    let site = Site::new().file("a.css", "x".repeat(16));
    let reply = site.get("/a.css").await;

    assert_eq!(reply.header("vary"), Some("accept-encoding"));
    assert!(!reply.has("content-encoding"));
  }

  #[tokio::test]
  async fn a_compressed_body_is_not_ranged() {
    // Current behaviour: compression is decided first and returns whole, so a
    // range on a gzip-able text file is answered with the entire compressed
    // body — and without `Accept-Ranges`, so a client has been told as much.
    let site = Site::new().file("a.txt", prose());
    let reply = site
      .send(
        Req::get("/a.txt")
          .header("accept-encoding", "gzip")
          .header("range", "bytes=0-9"),
      )
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.header("content-encoding"), Some("gzip"));
    assert!(!reply.has("accept-ranges"));
    assert_eq!(reply.gunzip(), prose().as_bytes());
  }

  #[tokio::test]
  async fn a_not_found_page_is_never_compressed() {
    let site = Site::new().not_found("404.html").file("404.html", prose());
    let reply = site
      .send(Req::get("/missing").header("accept-encoding", "gzip"))
      .await;

    assert_eq!(reply.status, 404);
    assert!(!reply.has("content-encoding"));
  }

  #[tokio::test]
  async fn head_of_a_compressed_body_declares_the_compressed_length() {
    let site = Site::new().file("a.txt", prose());
    let reply = site
      .send(Req::head("/a.txt").header("accept-encoding", "gzip"))
      .await;

    assert_eq!(reply.header("content-encoding"), Some("gzip"));
    assert!(reply.body.is_empty());
    assert!(reply.content_length().unwrap() < prose().len() as u64);
  }

  // ---- content types ----------------------------------------------------

  #[tokio::test]
  async fn guesses_the_content_type_from_the_name() {
    let site = Site::new()
      .file("a.css", "x")
      .file("a.js", "x")
      .file("a.png", "x")
      .file("a.bin", "x");

    assert_eq!(
      site.get("/a.css").await.header("content-type"),
      Some("text/css; charset=utf-8")
    );
    assert!(
      site
        .get("/a.js")
        .await
        .header("content-type")
        .unwrap()
        .contains("javascript")
    );
    assert_eq!(
      site.get("/a.png").await.header("content-type"),
      Some("image/png")
    );
    assert_eq!(
      site.get("/a.bin").await.header("content-type"),
      Some("application/octet-stream")
    );
  }

  // ---- the parts, on their own ------------------------------------------

  #[test]
  fn reads_a_closed_range() {
    assert_eq!(parse_range("bytes=2-5", 10), Range::Partial(2, 5));
    assert_eq!(parse_range("bytes=0-0", 10), Range::Partial(0, 0));
    assert_eq!(parse_range("bytes=0-9", 10), Range::Partial(0, 9));
  }

  #[test]
  fn reads_an_open_range() {
    assert_eq!(parse_range("bytes=7-", 10), Range::Partial(7, 9));
    assert_eq!(parse_range("bytes=0-", 10), Range::Partial(0, 9));
  }

  #[test]
  fn reads_a_suffix_range() {
    assert_eq!(parse_range("bytes=-3", 10), Range::Partial(7, 9));
    assert_eq!(parse_range("bytes=-99", 10), Range::Partial(0, 9));
    assert_eq!(parse_range("bytes=-10", 10), Range::Partial(0, 9));
  }

  #[test]
  fn tolerates_whitespace_around_the_spec() {
    assert_eq!(parse_range("bytes= 2 - 5 ", 10), Range::Partial(2, 5));
  }

  #[test]
  fn clamps_an_end_past_the_file() {
    assert_eq!(parse_range("bytes=5-99", 10), Range::Partial(5, 9));
  }

  #[test]
  fn rejects_a_range_that_starts_past_the_file() {
    assert_eq!(parse_range("bytes=10-12", 10), Range::Unsatisfiable);
    assert_eq!(parse_range("bytes=10-", 10), Range::Unsatisfiable);
    assert_eq!(parse_range("bytes=-0", 10), Range::Unsatisfiable);
  }

  #[test]
  fn rejects_a_range_that_runs_backwards() {
    assert_eq!(parse_range("bytes=5-3", 10), Range::Unsatisfiable);
  }

  #[test]
  fn falls_back_to_the_whole_file() {
    assert_eq!(parse_range("items=0-1", 10), Range::Whole);
    assert_eq!(parse_range("bytes=0-1,4-5", 10), Range::Whole);
    assert_eq!(parse_range("bytes=abc-", 10), Range::Whole);
    assert_eq!(parse_range("bytes=1-abc", 10), Range::Whole);
    assert_eq!(parse_range("bytes=nonsense", 10), Range::Whole);
    assert_eq!(parse_range("", 10), Range::Whole);
  }

  #[test]
  fn a_range_over_an_empty_file_is_unsatisfiable() {
    // There is no byte to hand back, so every spelling of a range over an empty
    // file is unsatisfiable — and `len - 1` must not be reached to say so.
    assert_eq!(parse_range("bytes=0-", 0), Range::Unsatisfiable);
    assert_eq!(parse_range("bytes=-5", 0), Range::Unsatisfiable);
    assert_eq!(parse_range("bytes=0-5", 0), Range::Unsatisfiable);
  }

  #[tokio::test]
  async fn a_range_request_for_an_empty_file_is_refused_not_fatal() {
    // The underflow was reachable from the wire, so the guard is pinned there
    // too: a debug build used to panic the connection task outright.
    let site = Site::new().file("empty.bin", "");
    let reply = site
      .send(Req::get("/empty.bin").header("Range", "bytes=0-"))
      .await;

    assert_eq!(reply.status, 416);
    assert_eq!(reply.header("content-range"), Some("bytes */0"));
  }

  #[test]
  fn an_etag_is_built_from_the_size_and_the_timestamp() {
    let epoch = UNIX_EPOCH;
    assert_eq!(etag(0, Some(epoch), None), "\"0-0\"");
    assert_eq!(
      etag(255, Some(epoch + std::time::Duration::from_secs(1)), None),
      "\"ff-3b9aca00\""
    );
  }

  #[test]
  fn an_etag_without_a_timestamp_still_has_a_shape() {
    // Some filesystems have no mtime to give; the size alone still changes when
    // the file does, most of the time.
    assert_eq!(etag(16, None, None), "\"10-0\"");
  }

  #[test]
  fn an_etag_names_its_encoding() {
    let epoch = Some(UNIX_EPOCH);
    assert_eq!(etag(0, epoch, Some(Encoding::Gzip)), "\"0-0-gz\"");
    assert_ne!(etag(0, epoch, None), etag(0, epoch, Some(Encoding::Gzip)));
  }

  #[test]
  fn an_etag_moves_with_either_half_of_what_it_is_made_of() {
    let epoch = Some(UNIX_EPOCH);
    let later = Some(UNIX_EPOCH + std::time::Duration::from_secs(1));

    assert_ne!(etag(1, epoch, None), etag(2, epoch, None));
    assert_ne!(etag(1, epoch, None), etag(1, later, None));
    assert_eq!(etag(1, epoch, None), etag(1, epoch, None));
  }

  #[test]
  fn a_validator_matches_itself_a_list_or_a_wildcard() {
    assert!(matches(Some("\"abc\""), "\"abc\""));
    assert!(matches(Some("*"), "\"abc\""));
    assert!(matches(Some("\"x\", \"abc\""), "\"abc\""));
    assert!(matches(Some("  \"abc\"  "), "\"abc\""));
  }

  #[test]
  fn a_validator_does_not_match_anything_else() {
    assert!(!matches(None, "\"abc\""));
    assert!(!matches(Some(""), "\"abc\""));
    assert!(!matches(Some("\"other\""), "\"abc\""));
    // Current behaviour: a weak validator is not unwrapped, so `W/"abc"` is a
    // miss. Nothing serv sends is weak, so nothing well-behaved sends one back.
    assert!(!matches(Some("W/\"abc\""), "\"abc\""));
  }

  #[test]
  fn tags_text_with_a_charset() {
    assert_eq!(
      content_type(Path::new("a.html")),
      "text/html; charset=utf-8"
    );
    assert_eq!(content_type(Path::new("a.css")), "text/css; charset=utf-8");
    assert_eq!(
      content_type(Path::new("a.svg")),
      "image/svg+xml; charset=utf-8"
    );
    assert_eq!(
      content_type(Path::new("a.json")),
      "application/json; charset=utf-8"
    );
    assert_eq!(content_type(Path::new("a.png")), "image/png");
    assert_eq!(content_type(Path::new("a.woff2")), "font/woff2");
  }

  #[test]
  fn an_unknown_name_is_a_stream_of_bytes() {
    assert_eq!(
      content_type(Path::new("LICENSE")),
      "application/octet-stream"
    );
    assert_eq!(
      content_type(Path::new("a.whatever")),
      "application/octet-stream"
    );
  }
}
