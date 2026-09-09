//! On-the-fly compression for the text formats that benefit from it.
//!
//! Nothing is stored: a response is compressed when it is sent, the same way it
//! is read. Only bodies small enough to sit in memory are considered, so a large
//! asset keeps its streaming path and its range support.

use std::io::{self, Write};

/// Above this, streaming the file uncompressed beats buffering it to compress.
pub const MAX_BODY: u64 = 8 * 1024 * 1024;

/// Below this, the framing costs more than the saving.
pub const MIN_BODY: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
  Brotli,
  Gzip,
}

impl Encoding {
  pub fn name(self) -> &'static str {
    match self {
      Encoding::Brotli => "br",
      Encoding::Gzip => "gzip",
    }
  }

  /// Suffix for the `ETag`, so a cached compressed body is never confused with
  /// the identity one it was made from.
  pub fn tag(self) -> &'static str {
    match self {
      Encoding::Brotli => "-br",
      Encoding::Gzip => "-gz",
    }
  }
}

/// Pick an encoding from `Accept-Encoding`, preferring brotli.
pub fn negotiate(header: Option<&str>) -> Option<Encoding> {
  let header = header?;
  let mut brotli = false;
  let mut gzip = false;

  for entry in header.split(',') {
    let mut parts = entry.split(';');
    let name = parts.next()?.trim();
    // `q=0` is a refusal, and the only q value worth reading here.
    let refused = parts.any(|param| {
      let param = param.trim();
      param
        .strip_prefix("q=")
        .is_some_and(|q| q.parse::<f32>().is_ok_and(|q| q == 0.0))
    });
    if refused {
      continue;
    }
    match name {
      "br" => brotli = true,
      "gzip" => gzip = true,
      _ => {}
    }
  }

  match (brotli, gzip) {
    (true, _) => Some(Encoding::Brotli),
    (false, true) => Some(Encoding::Gzip),
    _ => None,
  }
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
    Encoding::Brotli => {
      let mut out = Vec::new();
      // Quality 4 with a 4 MiB window: the point on the curve where a dev
      // server still feels instant.
      let mut writer = brotli::CompressorWriter::new(&mut out, 8192, 4, 22);
      writer.write_all(data)?;
      drop(writer);
      Ok(out)
    }
  }
}
