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

#[cfg(test)]
mod tests {
  use super::*;

  fn root() -> PathBuf {
    PathBuf::from("/srv")
  }

  #[test]
  fn maps_a_plain_path() {
    assert_eq!(
      resolve(&root(), "/a/b.txt"),
      Some(PathBuf::from("/srv/a/b.txt"))
    );
  }

  #[test]
  fn ignores_empty_and_dot_segments() {
    assert_eq!(
      resolve(&root(), "//a/./b/"),
      Some(PathBuf::from("/srv/a/b"))
    );
  }

  #[test]
  fn decodes_percent_escapes() {
    assert_eq!(
      resolve(&root(), "/my%20file.txt"),
      Some(PathBuf::from("/srv/my file.txt"))
    );
  }

  #[test]
  fn refuses_to_climb_out_of_the_root() {
    assert_eq!(resolve(&root(), "/../etc/passwd"), None);
    assert_eq!(resolve(&root(), "/a/../../etc"), None);
  }

  #[test]
  fn climbing_within_the_root_is_fine() {
    assert_eq!(
      resolve(&root(), "/a/b/../c"),
      Some(PathBuf::from("/srv/a/c"))
    );
  }

  #[test]
  fn refuses_separators_smuggled_through_encoding() {
    assert_eq!(resolve(&root(), "/a%2f..%2f..%2fetc"), None);
    assert_eq!(resolve(&root(), "/a%5cb"), None);
  }
}
