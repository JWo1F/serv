use std::io;
use std::path::Path;

use damask::Component;

use crate::pages::icons::Kind;
use crate::pages::{human_size, human_time};

/// The directory index: a folder presented as the contents page of a book.
#[derive(Component)]
pub struct Listing {
  /// Trail of links from the served root down to this directory.
  pub crumbs: Vec<Crumb>,
  /// `None` at the root, where there is nowhere to go up to.
  pub parent: Option<String>,
  pub entries: Vec<Entry>,
  /// Absolute path of the served directory, for the colophon.
  pub root: String,
}

pub struct Crumb {
  pub label: String,
  pub href: String,
}

pub struct Entry {
  pub name: String,
  pub href: String,
  pub is_dir: bool,
  pub kind: Kind,
  pub size: String,
  pub modified: String,
}

impl Entry {
  pub fn icon(&self) -> &'static str {
    self.kind.icon()
  }
}

impl Listing {
  pub fn style(&self) -> &'static str {
    crate::pages::STYLE
  }

  pub fn tally(&self) -> String {
    let dirs = self.entries.iter().filter(|e| e.is_dir).count();
    let files = self.entries.len() - dirs;
    format!("{dirs} folders · {files} files")
  }
}

/// Read a directory into an index: folders first, then files, each alphabetical.
pub async fn read(dir: &Path, url_path: &str, root: &Path) -> io::Result<Listing> {
  let mut entries = Vec::new();
  let mut reader = tokio::fs::read_dir(dir).await?;

  while let Some(item) = reader.next_entry().await? {
    let meta = match item.metadata().await {
      Ok(meta) => meta,
      // A symlink that dangles is not worth failing the whole page over.
      Err(_) => continue,
    };
    let name = item.file_name().to_string_lossy().into_owned();
    let is_dir = meta.is_dir();

    entries.push(Entry {
      href: format!("{}{}", encode(&name), if is_dir { "/" } else { "" }),
      kind: if is_dir {
        Kind::Folder
      } else {
        Kind::of(&name)
      },
      // A folder has no size worth printing, so the column stays empty.
      size: if is_dir {
        String::new()
      } else {
        human_size(meta.len())
      },
      modified: meta.modified().map(human_time).unwrap_or_default(),
      name: if is_dir { format!("{name}/") } else { name },
      is_dir,
    });
  }

  entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

  Ok(Listing {
    crumbs: crumbs(url_path, root),
    parent: (url_path != "/").then(|| "../".to_string()),
    entries,
    root: root.display().to_string(),
  })
}

/// The trail from the served folder down to `url_path`. The first crumb wears
/// the folder's own name rather than a bare slash, so the trail says which tree
/// you are in — a root with no name of its own keeps the slash.
pub fn crumbs(url_path: &str, root: &Path) -> Vec<Crumb> {
  let name = root
    .file_name()
    .map(|name| format!("{}/", name.to_string_lossy()))
    .unwrap_or_else(|| "/".to_string());

  let mut crumbs = vec![Crumb {
    label: name,
    href: "/".to_string(),
  }];
  let mut href = String::from("/");

  for segment in url_path.split('/').filter(|s| !s.is_empty()) {
    href.push_str(segment);
    href.push('/');
    crumbs.push(Crumb {
      label: format!("{segment}/"),
      href: href.clone(),
    });
  }
  crumbs
}

/// Percent-encode a file name for use as a single URL path segment.
fn encode(name: &str) -> String {
  use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

  const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

  utf8_percent_encode(name, SEGMENT).to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  /// A root with the given entries; a name ending in `/` becomes a directory.
  fn tree(names: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in names {
      match name.strip_suffix('/') {
        Some(folder) => fs::create_dir(dir.path().join(folder)).unwrap(),
        None => fs::write(dir.path().join(name), "").unwrap(),
      }
    }
    dir
  }

  async fn listing(dir: &TempDir, url_path: &str) -> Listing {
    read(dir.path(), url_path, dir.path()).await.unwrap()
  }

  fn names(listing: &Listing) -> Vec<&str> {
    listing.entries.iter().map(|e| e.name.as_str()).collect()
  }

  #[tokio::test]
  async fn puts_folders_first_then_sorts_alphabetically() {
    let dir = tree(&["b.txt", "a.txt", "zeta/", "alpha/"]);
    let page = listing(&dir, "/").await;

    assert_eq!(names(&page), ["alpha/", "zeta/", "a.txt", "b.txt"]);
  }

  #[tokio::test]
  async fn sorts_by_byte_order_so_capitals_come_first() {
    // Plain `str` ordering, which puts `README` above `assets` the way `ls`
    // does in the C locale.
    let dir = tree(&["alpha.txt", "Beta.txt"]);
    let page = listing(&dir, "/").await;

    assert_eq!(names(&page), ["Beta.txt", "alpha.txt"]);
  }

  #[tokio::test]
  async fn marks_folders_with_a_trailing_slash_and_no_size() {
    let dir = tree(&["docs/", "a.txt"]);
    let page = listing(&dir, "/").await;

    let docs = &page.entries[0];
    assert!(docs.is_dir);
    assert_eq!(docs.name, "docs/");
    assert_eq!(docs.href, "docs/");
    assert_eq!(docs.size, "");
    assert_eq!(docs.kind, Kind::Folder);

    let file = &page.entries[1];
    assert!(!file.is_dir);
    assert_eq!(file.href, "a.txt");
    assert_eq!(file.size, "0 B");
    assert!(!file.modified.is_empty());
  }

  #[tokio::test]
  async fn reports_a_files_size() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("big.bin"), vec![0u8; 2048]).unwrap();
    let page = listing(&dir, "/").await;

    assert_eq!(page.entries[0].size, "2.0 kB");
  }

  #[tokio::test]
  async fn picks_an_icon_per_entry() {
    let dir = tree(&["page.html", "app.css", "main.js", "notes.txt", "sub/"]);
    let page = listing(&dir, "/").await;
    let kinds: Vec<Kind> = page.entries.iter().map(|e| e.kind).collect();

    assert_eq!(
      kinds,
      [Kind::Folder, Kind::Css, Kind::Js, Kind::File, Kind::Html]
    );
  }

  #[tokio::test]
  async fn an_empty_directory_lists_nothing() {
    let dir = tree(&[]);
    let page = listing(&dir, "/").await;

    assert!(page.entries.is_empty());
    assert_eq!(page.tally(), "0 folders · 0 files");
  }

  #[tokio::test]
  async fn counts_folders_and_files_separately() {
    let dir = tree(&["a/", "b/", "c.txt"]);
    let page = listing(&dir, "/").await;

    assert_eq!(page.tally(), "2 folders · 1 files");
  }

  #[tokio::test]
  async fn there_is_nowhere_to_go_up_to_from_the_root() {
    let dir = tree(&[]);
    assert_eq!(listing(&dir, "/").await.parent, None);
    assert_eq!(
      listing(&dir, "/docs/").await.parent,
      Some("../".to_string())
    );
  }

  #[tokio::test]
  async fn names_the_served_directory_for_the_colophon() {
    let dir = tree(&[]);
    let page = listing(&dir, "/").await;

    assert_eq!(page.root, dir.path().display().to_string());
  }

  #[tokio::test]
  async fn renders_its_entries_and_its_stylesheet() {
    let dir = tree(&["readme.md", "docs/"]);
    let page = listing(&dir, "/").await;
    let html = page.render();

    assert!(html.contains("readme.md"));
    assert!(html.contains("docs/"));
    // The stylesheet is inlined, not linked, so the page works offline.
    assert!(html.contains(crate::pages::STYLE));
    assert!(!page.style().is_empty());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_dangling_symlink_is_listed_as_an_ordinary_entry() {
    // The metadata behind a `DirEntry` is read without following the link, so a
    // dangling one still has metadata and is listed. The `continue` guarding
    // that read is for the rarer failures — a directory that became unreadable
    // between the scan and the stat.
    let dir = tree(&["real.txt"]);
    std::os::unix::fs::symlink(dir.path().join("gone.txt"), dir.path().join("broken")).unwrap();
    let page = listing(&dir, "/").await;

    assert_eq!(names(&page), ["broken", "real.txt"]);
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_symlink_to_a_directory_is_listed_as_a_file() {
    // Same cause: the link itself is stat-ed, so it is not marked as a folder.
    // Following it still works — the link has no trailing slash, so the request
    // is redirected to one and served from there.
    let dir = tree(&["docs/"]);
    std::os::unix::fs::symlink(dir.path().join("docs"), dir.path().join("guide")).unwrap();
    let page = listing(&dir, "/").await;

    assert_eq!(names(&page), ["docs/", "guide"]);
    assert!(!page.entries[1].is_dir);
  }

  #[tokio::test]
  async fn reading_a_directory_that_is_not_there_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");

    assert!(read(&missing, "/nope/", dir.path()).await.is_err());
  }

  #[tokio::test]
  async fn percent_encodes_names_that_would_break_a_url() {
    let dir = tree(&["my file.txt", "a#b.txt", "q?.txt", "100%.txt"]);
    let page = listing(&dir, "/").await;
    let hrefs: Vec<&str> = page.entries.iter().map(|e| e.href.as_str()).collect();

    assert!(hrefs.contains(&"my%20file.txt"));
    assert!(hrefs.contains(&"a%23b.txt"));
    assert!(hrefs.contains(&"q%3F.txt"));
    assert!(hrefs.contains(&"100%25.txt"));
    // The label keeps the name as it is on disk; only the link is encoded.
    assert!(names(&page).contains(&"my file.txt"));
  }

  #[test]
  fn encodes_a_segment_without_touching_what_is_already_safe() {
    assert_eq!(encode("plain.txt"), "plain.txt");
    assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
    assert_eq!(encode("my file.txt"), "my%20file.txt");
    assert_eq!(encode("a#b"), "a%23b");
    assert_eq!(encode("a?b"), "a%3Fb");
    assert_eq!(encode("50%"), "50%25");
    assert_eq!(encode("a/b"), "a%2Fb");
    assert_eq!(encode("a\\b"), "a%5Cb");
    assert_eq!(encode("<i>"), "%3Ci%3E");
    assert_eq!(encode("a\"b"), "a%22b");
    assert_eq!(encode("a`b^c{d}e|f"), "a%60b%5Ec%7Bd%7De%7Cf");
  }

  #[test]
  fn encodes_unicode_as_utf8_bytes() {
    assert_eq!(
      encode("привет.txt"),
      "%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82.txt"
    );
    assert_eq!(encode("café"), "caf%C3%A9");
    assert_eq!(encode("🙂"), "%F0%9F%99%82");
  }

  #[test]
  fn the_root_gets_a_single_crumb_named_after_the_folder() {
    let trail = crumbs("/", Path::new("/home/alex/site"));
    assert_eq!(trail.len(), 1);
    assert_eq!(trail[0].label, "site/");
    assert_eq!(trail[0].href, "/");
  }

  #[test]
  fn a_root_with_no_name_of_its_own_stays_a_slash() {
    // `serv /` has no folder name to show; the slash is the only honest label.
    assert_eq!(crumbs("/", Path::new("/"))[0].label, "/");
  }

  #[test]
  fn a_nested_path_gets_a_crumb_per_segment() {
    let built = crumbs("/a/b/c/", Path::new("/home/alex/site"));
    let trail: Vec<(&str, &str)> = built
      .iter()
      .map(|c| (c.label.as_str(), c.href.as_str()))
      .collect();

    assert_eq!(
      trail,
      [
        ("site/", "/"),
        ("a/", "/a/"),
        ("b/", "/a/b/"),
        ("c/", "/a/b/c/"),
      ]
    );
  }

  #[test]
  fn crumb_links_are_absolute_and_end_in_a_slash() {
    // Every crumb points at a directory, so each href must be one a browser can
    // follow from anywhere in the tree.
    for crumb in crumbs("/docs/guide/", Path::new("/home/alex/site")) {
      assert!(crumb.href.starts_with('/'));
      assert!(crumb.href.ends_with('/'));
    }
  }
}
