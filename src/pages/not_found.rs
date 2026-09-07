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
