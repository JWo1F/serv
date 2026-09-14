//! On-the-fly gzip for the text formats that benefit from it.
//!
//! Nothing is stored: a response is compressed when it is sent, the same way it
//! is read. Only bodies small enough to sit in memory are considered, so a large
//! asset keeps its streaming path and its range support.
//!
//! gzip is the whole list, and every browser accepts it.

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

  fn unzip(packed: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(packed)
      .read_to_end(&mut out)
      .unwrap();
    out
  }

  #[test]
  fn takes_gzip_when_offered() {
    assert_eq!(negotiate(Some("gzip")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip, deflate")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip, deflate, br")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("br, gzip")), Some(Encoding::Gzip));
  }

  #[test]
  fn tolerates_the_whitespace_clients_actually_send() {
    assert_eq!(negotiate(Some("  gzip  ")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("deflate,gzip")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip ; q=1.0")), Some(Encoding::Gzip));
  }

  #[test]
  fn honours_a_refusal() {
    assert_eq!(negotiate(Some("gzip;q=0")), None);
    assert_eq!(negotiate(Some("br, gzip;q=0")), None);
    assert_eq!(negotiate(Some("gzip;q=0.0")), None);
    assert_eq!(negotiate(Some("gzip;q=0.000")), None);
    assert_eq!(negotiate(Some("gzip; q=0")), None);
  }

  #[test]
  fn any_q_above_zero_is_acceptance() {
    // The only q value worth reading is the refusal; a preference order between
    // encodings is moot when gzip is the only one on offer.
    assert_eq!(negotiate(Some("gzip;q=0.001")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip;q=0.5")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip;q=1")), Some(Encoding::Gzip));
  }

  #[test]
  fn a_malformed_q_is_not_a_refusal() {
    assert_eq!(negotiate(Some("gzip;q=")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip;q=nonsense")), Some(Encoding::Gzip));
    assert_eq!(negotiate(Some("gzip;level=9")), Some(Encoding::Gzip));
  }

  #[test]
  fn a_refusal_does_not_poison_a_later_acceptance() {
    // Nonsense to send, but each entry is judged on its own, and a list that
    // still contains a plain `gzip` is taken as offering it.
    assert_eq!(negotiate(Some("gzip;q=0, gzip")), Some(Encoding::Gzip));
  }

  #[test]
  fn identity_when_gzip_is_not_offered() {
    assert_eq!(negotiate(None), None);
    assert_eq!(negotiate(Some("")), None);
    assert_eq!(negotiate(Some("identity")), None);
    assert_eq!(negotiate(Some("br")), None);
    assert_eq!(negotiate(Some("deflate, br, zstd")), None);
    // A name that merely contains "gzip" is a different encoding.
    assert_eq!(negotiate(Some("x-gzip")), None);
  }

  #[test]
  fn matches_the_encoding_name_literally() {
    // Current behaviour: the comparison is case-sensitive and the `*` wildcard
    // is not expanded, so both of these read as "gzip was not offered". Every
    // browser sends a lowercase, explicit `gzip`, so neither is reachable in
    // practice.
    assert_eq!(negotiate(Some("GZIP")), None);
    assert_eq!(negotiate(Some("*")), None);
  }

  #[test]
  fn compresses_only_what_benefits() {
    assert!(worthwhile("text/css; charset=utf-8"));
    assert!(worthwhile("text/html"));
    assert!(worthwhile("text/plain"));
    assert!(worthwhile("application/json"));
    assert!(worthwhile("application/javascript"));
    assert!(worthwhile("application/manifest+json"));
    assert!(worthwhile("application/wasm"));
    assert!(worthwhile("application/xml"));
    assert!(worthwhile("application/xhtml+xml"));
    assert!(worthwhile("image/svg+xml"));
  }

  #[test]
  fn leaves_already_compressed_formats_alone() {
    assert!(!worthwhile("image/png"));
    assert!(!worthwhile("image/jpeg"));
    assert!(!worthwhile("image/webp"));
    assert!(!worthwhile("video/mp4"));
    assert!(!worthwhile("audio/mpeg"));
    assert!(!worthwhile("application/zip"));
    assert!(!worthwhile("application/octet-stream"));
    assert!(!worthwhile("font/woff2"));
  }

  #[test]
  fn reads_past_the_parameters_and_the_whitespace() {
    assert!(worthwhile("  text/html ; charset=utf-8 "));
    assert!(!worthwhile(""));
  }

  #[test]
  fn the_thresholds_are_one_kib_and_eight_mib() {
    assert_eq!(MIN_BODY, 1024);
    assert_eq!(MAX_BODY, 8 * 1024 * 1024);
  }

  #[test]
  fn names_itself_for_the_wire_and_for_the_etag() {
    assert_eq!(Encoding::Gzip.name(), "gzip");
    assert_eq!(Encoding::Gzip.tag(), "-gz");
  }

  #[test]
  fn round_trips_through_gzip() {
    let data = b"hello hello hello hello".repeat(64);
    let packed = encode(&data, Encoding::Gzip).unwrap();
    assert!(packed.len() < data.len());
    assert_eq!(unzip(&packed), data);
  }

  #[test]
  fn round_trips_bytes_that_are_not_text() {
    let data: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    let packed = encode(&data, Encoding::Gzip).unwrap();
    assert_eq!(unzip(&packed), data);
  }

  #[test]
  fn encodes_an_empty_body() {
    let packed = encode(b"", Encoding::Gzip).unwrap();
    // A gzip member is never empty: it is all header and trailer.
    assert!(!packed.is_empty());
    assert_eq!(unzip(&packed), Vec::<u8>::new());
  }
}
