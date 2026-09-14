use std::path::Path;

use damask::Component;

use crate::markdown;
use crate::pages::listing::{Crumb, crumbs};

/// A markdown file, set on the same sheet as the other two pages.
#[derive(Component)]
pub struct Document {
  /// The first heading, or the file name — the browser tab's label.
  pub title: String,
  /// The rendered markdown. Inserted raw; it is HTML by the time it gets here.
  pub body: String,
  /// Trail of links from the served root down to this file.
  pub crumbs: Vec<Crumb>,
  /// Absolute path of the served directory, for the colophon.
  pub root: String,
}

impl Document {
  pub fn new(source: &str, name: &str, url_path: &str, root: &Path, clean_urls: bool) -> Self {
    let rendered = markdown::render(source, name, clean_urls);
    Self {
      title: rendered.title,
      body: rendered.body,
      crumbs: crumbs(url_path, root),
      root: root.display().to_string(),
    }
  }

  pub fn style(&self) -> &'static str {
    crate::pages::STYLE
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn page(source: &str, url_path: &str) -> String {
    Document::new(source, "doc.md", url_path, Path::new("/site"), true).render()
  }

  #[test]
  fn the_rendered_markdown_is_the_body_of_the_page() {
    let html = page("# Reading\n\nsome prose", "/reading");
    assert!(html.contains("<h1 id=\"reading\">Reading</h1>"), "{html}");
    assert!(html.contains("<p>some prose</p>"), "{html}");
  }

  #[test]
  fn the_heading_becomes_the_tab_title() {
    assert!(page("# Reading\n", "/reading").contains("<title>Reading</title>"));
  }

  #[test]
  fn the_body_is_not_escaped_into_visible_tags() {
    // `{@html}` rather than `{}`: the body is already HTML.
    let html = page("**bold**", "/x");
    assert!(html.contains("<strong>bold</strong>"), "{html}");
    assert!(!html.contains("&lt;strong&gt;"), "{html}");
  }

  #[test]
  fn the_title_is_escaped_because_it_is_text() {
    let html = page("# Tom & Jerry\n", "/x");
    assert!(html.contains("<title>Tom &amp; Jerry</title>"), "{html}");
  }

  #[test]
  fn markup_in_a_heading_never_reaches_the_title() {
    // The title is built from text events only, so inline HTML is dropped on
    // the way rather than escaped — a `<script>` cannot arrive in the head.
    let html = page("# a <script> tag\n", "/x");
    assert!(html.contains("<title>a  tag</title>"), "{html}");
  }

  #[test]
  fn the_page_carries_a_trail_back_up() {
    let html = page("text", "/docs/guide");
    assert!(html.contains(r#"href="/docs/""#), "{html}");
  }

  #[test]
  fn the_stylesheet_is_inlined_so_the_page_works_offline() {
    let html = page("text", "/x");
    assert!(html.contains("--vermilion"), "{html}");
    assert!(!html.contains("<link rel=\"stylesheet\""), "{html}");
  }
}
