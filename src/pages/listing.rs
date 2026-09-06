use std::io;
use std::path::Path;

use damask::Component;

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
  pub size: String,
  pub modified: String,
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
      name: if is_dir { format!("{name}/") } else { name },
      is_dir,
      size: if is_dir {
        "—".to_string()
      } else {
        human_size(meta.len())
      },
      modified: meta.modified().map(human_time).unwrap_or_default(),
    });
  }

  entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

  Ok(Listing {
    crumbs: crumbs(url_path),
    parent: (url_path != "/").then(|| "../".to_string()),
    entries,
    root: root.display().to_string(),
  })
}

fn crumbs(url_path: &str) -> Vec<Crumb> {
  let mut crumbs = vec![Crumb {
    label: "/".to_string(),
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
