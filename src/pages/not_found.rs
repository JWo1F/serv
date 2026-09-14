use damask::Component;

/// The 404: a missing sort in the printer's forme.
#[derive(Component)]
pub struct NotFound {
  /// The address that was asked for, shown back verbatim.
  pub path: String,
  /// Where "up one level" goes.
  pub parent: String,
  /// Host and port, so the address reads the way the browser shows it.
  pub host: String,
}

impl NotFound {
  pub fn new(url_path: &str, host: &str) -> Self {
    Self {
      parent: parent_of(url_path),
      path: url_path.to_string(),
      host: host.to_string(),
    }
  }

  pub fn style(&self) -> &'static str {
    crate::pages::STYLE
  }
}

fn parent_of(url_path: &str) -> String {
  let trimmed = url_path.trim_end_matches('/');
  match trimmed.rfind('/') {
    Some(cut) => trimmed[..=cut].to_string(),
    None => "/".to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_parent_of_a_page_is_its_directory() {
    assert_eq!(parent_of("/a/b.html"), "/a/");
    assert_eq!(parent_of("/a/b/c"), "/a/b/");
    assert_eq!(parent_of("/top.html"), "/");
  }

  #[test]
  fn a_trailing_slash_is_ignored_before_climbing() {
    // `/a/b/` is a directory that does not exist; "up" from it is `/a/`, not
    // itself.
    assert_eq!(parent_of("/a/b/"), "/a/");
    assert_eq!(parent_of("/a/"), "/");
  }

  #[test]
  fn the_root_is_its_own_parent() {
    assert_eq!(parent_of("/"), "/");
    assert_eq!(parent_of("//"), "/");
    assert_eq!(parent_of(""), "/");
  }

  #[test]
  fn a_path_with_no_slash_at_all_climbs_to_the_root() {
    assert_eq!(parent_of("nope"), "/");
  }

  #[test]
  fn shows_the_address_back_verbatim() {
    // Whatever was typed is what the page repeats, escaping aside — it is the
    // one piece of information the reader came for.
    let page = NotFound::new("/missing/page.html", "127.0.0.1:8010");

    assert_eq!(page.path, "/missing/page.html");
    assert_eq!(page.parent, "/missing/");
    assert_eq!(page.host, "127.0.0.1:8010");
  }

  #[test]
  fn renders_the_path_the_host_and_the_inlined_stylesheet() {
    let page = NotFound::new("/missing/page.html", "localhost:8010");
    let html = page.render();

    assert!(html.contains("/missing/page.html"));
    assert!(html.contains("localhost:8010"));
    assert!(html.contains(crate::pages::STYLE));
    assert!(!page.style().is_empty());
  }

  #[test]
  fn escapes_markup_in_the_path_it_echoes() {
    // The path comes straight off the wire, so the template has to be the thing
    // that makes it safe to print.
    let page = NotFound::new("/<script>alert(1)</script>", "localhost");
    let html = page.render();

    assert!(!html.contains("<script>alert(1)</script>"));
  }
}
