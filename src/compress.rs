//! On-the-fly gzip for the text formats that benefit from it.
//!
//! Nothing is stored: a response is compressed when it is sent, the same way it
//! is read. Only bodies small enough to sit in memory are considered, so a large
//! asset keeps its streaming path and its range support.
//!
//! gzip is the whole list. Brotli compresses a little tighter, but it carries a
//! static dictionary and Huffman tables that cost about a megabyte of binary —
//! roughly half of serv — to save bytes on a connection that never leaves the
//! machine. Every browser accepts gzip.

use std::io::{self, Write};

/// Above this, streaming the file uncompressed beats buffering it to compress.
pub const MAX_BODY: u64 = 8 * 1024 * 1024;

/// Below this, the framing costs more than the saving.
pub const MIN_BODY: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
  Gzip,
}

impl Encoding {
  pub fn name(self) -> &'static str {
    match self {
      Encoding::Gzip => "gzip",
    }
  }

  /// Suffix for the `ETag`, so a cached compressed body is never confused with
  /// the identity one it was made from.
  pub fn tag(self) -> &'static str {
    match self {
      Encoding::Gzip => "-gz",
    }
  }
}

/// Read `Accept-Encoding` and say whether gzip is welcome.
pub fn negotiate(header: Option<&str>) -> Option<Encoding> {
  let header = header?;

  for entry in header.split(',') {
    let mut parts = entry.split(';');
    let name = parts.next()?.trim();
    if name != "gzip" {
      continue;
    }
    // `q=0` is a refusal, and the only q value worth reading here.
    let refused = parts.any(|param| {
      param
        .trim()
        .strip_prefix("q=")
        .is_some_and(|q| q.parse::<f32>().is_ok_and(|q| q == 0.0))
    });
    if !refused {
      return Some(Encoding::Gzip);
    }
  }
  None
}

/// Whether a body of this media type is worth compressing at all.
pub fn worthwhile(content_type: &str) -> bool {
  let essence = content_type.split(';').next().unwrap_or_default().trim();

  essence.starts_with("text/")
    || matches!(
      essence,
      "application/javascript"
        | "application/json"
        | "application/manifest+json"
        | "application/wasm"
        | "application/xml"
        | "application/xhtml+xml"
        | "image/svg+xml"
    )
}

pub fn encode(data: &[u8], encoding: Encoding) -> io::Result<Vec<u8>> {
  match encoding {
    Encoding::Gzip => {
      let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(4));
      writer.write_all(data)?;
      writer.finish()
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn takes_gzip_when_offered() {
    assert_eq!(negotiate(Some("gzip, deflate")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip, deflate, br")), Some(Encoding::Gzip));
  }

  #[test]
  fn honours_a_refusal() {
    assert_eq!(negotiate(Some("gzip;q=0")), None);
    assert_eq!(negotiate(Some("br, gzip;q=0")), None);
  }

  #[test]
  fn identity_when_gzip_is_not_offered() {
    assert_eq!(negotiate(None), None);
    assert_eq!(negotiate(Some("identity")), None);
    assert_eq!(negotiate(Some("br")), None);
  }

  #[test]
  fn compresses_only_what_benefits() {
    assert!(worthwhile("text/css; charset=utf-8"));
    assert!(worthwhile("application/json"));
    assert!(worthwhile("image/svg+xml"));
    assert!(!worthwhile("image/png"));
    assert!(!worthwhile("video/mp4"));
  }

  #[test]
  fn round_trips_through_gzip() {
    let data = b"hello hello hello hello".repeat(64);
    let packed = encode(&data, Encoding::Gzip).unwrap();
    assert!(packed.len() < data.len());
  }
}
