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
  fn the_root_path_is_the_root_itself() {
    assert_eq!(resolve(&root(), "/"), Some(root()));
    // A handler never sees an empty path over HTTP/1.1, but the function is
    // total and should not be the thing that decides otherwise.
    assert_eq!(resolve(&root(), ""), Some(root()));
    assert_eq!(resolve(&root(), "///"), Some(root()));
  }

  #[test]
  fn ignores_empty_and_dot_segments() {
    assert_eq!(
      resolve(&root(), "//a/./b/"),
      Some(PathBuf::from("/srv/a/b"))
    );
    assert_eq!(resolve(&root(), "/./"), Some(root()));
  }

  #[test]
  fn a_relative_url_path_is_still_read_below_the_root() {
    // Segments are joined onto the root regardless of a leading slash, so a
    // request line that arrived without one cannot escape by that route.
    assert_eq!(resolve(&root(), "a/b"), Some(PathBuf::from("/srv/a/b")));
  }

  #[test]
  fn a_relative_root_stays_relative() {
    assert_eq!(
      resolve(Path::new("site"), "/a.txt"),
      Some(PathBuf::from("site/a.txt"))
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
  fn decodes_unicode_names() {
    assert_eq!(
      resolve(&root(), "/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82.txt"),
      Some(PathBuf::from("/srv/привет.txt"))
    );
    // Already-decoded UTF-8 in the path is passed through unchanged.
    assert_eq!(
      resolve(&root(), "/café/naïve.txt"),
      Some(PathBuf::from("/srv/café/naïve.txt"))
    );
  }

  #[test]
  fn refuses_invalid_utf8_escapes() {
    assert_eq!(resolve(&root(), "/%FF%FE"), None);
  }

  #[test]
  fn refuses_to_climb_out_of_the_root() {
    assert_eq!(resolve(&root(), "/../etc/passwd"), None);
    assert_eq!(resolve(&root(), "/a/../../etc"), None);
    assert_eq!(resolve(&root(), "/.."), None);
    // The count is what matters, not where the climb sits in the path.
    assert_eq!(resolve(&root(), "/a/b/../../../c"), None);
  }

  #[test]
  fn refuses_a_percent_encoded_dot_dot() {
    // `%2e%2e` decodes to `..` before the segment is classified, so the climb
    // is counted rather than treated as a literal file name.
    assert_eq!(resolve(&root(), "/%2e%2e/etc"), None);
    assert_eq!(resolve(&root(), "/a/%2E%2E/%2E%2E/etc"), None);
  }

  #[test]
  fn climbing_within_the_root_is_fine() {
    assert_eq!(
      resolve(&root(), "/a/b/../c"),
      Some(PathBuf::from("/srv/a/c"))
    );
    assert_eq!(resolve(&root(), "/a/.."), Some(root()));
  }

  #[test]
  fn refuses_separators_smuggled_through_encoding() {
    assert_eq!(resolve(&root(), "/a%2f..%2f..%2fetc"), None);
    assert_eq!(resolve(&root(), "/a%2F..%2Fetc"), None);
    assert_eq!(resolve(&root(), "/a%5cb"), None);
    assert_eq!(resolve(&root(), "/a%5Cb"), None);
  }

  #[test]
  fn refuses_an_embedded_nul() {
    // A truncating NUL would let `/safe.txt%00.png` reach a different file
    // than the extension suggests, so the whole request is refused.
    assert_eq!(resolve(&root(), "/a%00b"), None);
  }

  #[test]
  fn dots_are_only_special_as_a_whole_segment() {
    assert_eq!(resolve(&root(), "/...."), Some(PathBuf::from("/srv/....")),);
    assert_eq!(
      resolve(&root(), "/a/..b/c"),
      Some(PathBuf::from("/srv/a/..b/c")),
    );
    assert_eq!(
      resolve(&root(), "/.hidden"),
      Some(PathBuf::from("/srv/.hidden")),
    );
  }

  #[test]
  fn a_query_string_is_not_this_functions_business() {
    // `Uri::path()` has already split the query off; anything left is a name.
    assert_eq!(
      resolve(&root(), "/a?b=1"),
      Some(PathBuf::from("/srv/a?b=1"))
    );
  }

  #[test]
  fn a_path_inside_the_root_is_within_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let file = root.join("page.html");
    std::fs::write(&file, "hi").unwrap();

    assert!(is_within(&root, &file));
    assert!(is_within(&root, &root));
  }

  #[test]
  fn a_path_that_is_not_on_disk_is_allowed_through() {
    // `resolve` has already proved it cannot escape, and the caller is about to
    // fail to open it anyway.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    assert!(is_within(&root, &root.join("missing.html")));
  }

  #[cfg(unix)]
  #[test]
  fn a_symlink_pointing_out_of_the_root_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "shh").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let link = root.join("escape.txt");
    std::os::unix::fs::symlink(&secret, &link).unwrap();

    assert!(!is_within(&root, &link));
  }

  #[cfg(unix)]
  #[test]
  fn a_symlink_staying_inside_the_root_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let target = root.join("real.txt");
    std::fs::write(&target, "hi").unwrap();
    let link = root.join("alias.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(is_within(&root, &link));
  }

  #[cfg(unix)]
  #[test]
  fn a_dangling_symlink_cannot_be_canonicalised_so_it_passes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let link = root.join("dangling.txt");
    std::os::unix::fs::symlink(root.join("nowhere.txt"), &link).unwrap();

    assert!(is_within(&root, &link));
  }
}
