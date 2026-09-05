use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// Map a request path onto a path below `root`.
///
/// Returns `None` for anything that tries to leave the served directory, is not
/// valid UTF-8 once decoded, or smuggles a separator through percent-encoding.
pub fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
  let mut segments: Vec<String> = Vec::new();

  for raw in url_path.split('/') {
    if raw.is_empty() {
      continue;
    }
    let segment = percent_decode_str(raw).decode_utf8().ok()?.into_owned();

    match segment.as_str() {
      "." => continue,
      ".." => {
        segments.pop()?;
        continue;
      }
      _ => {}
    }

    if segment.contains(['/', '\\', '\0']) {
      return None;
    }
    segments.push(segment);
  }

  let mut path = root.to_path_buf();
  path.extend(segments);
  Some(path)
}

/// Second line of defence: a symlink inside the root may still point outside it.
pub fn is_within(root: &Path, path: &Path) -> bool {
  match path.canonicalize() {
    Ok(real) => real.starts_with(root),
    // Not yet on disk, so `resolve` above is the only guarantee we need.
    Err(_) => true,
  }
}
