use std::path::PathBuf;

/// Everything the request handler needs, resolved once at startup.
#[derive(Debug)]
pub struct Config {
  /// Canonical directory being served. Pointed at a single file, this is the
  /// folder that file sits in — paths still need something to be relative to.
  pub root: PathBuf,
  /// The one file to serve at `/`, when serv was pointed at a file rather than
  /// a directory. Nothing else is served while this is set.
  pub only: Option<PathBuf>,
  /// Single-page app entry point, when `--spa` is in play.
  pub spa: Option<PathBuf>,
  /// Require `.html` in URLs instead of stripping it.
  pub ext: bool,
  /// Page to serve for a miss, when `--not-found` is in play.
  pub not_found: Option<PathBuf>,
  /// Render `.md` files as pages, when `--markdown` is in play.
  pub markdown: bool,
}
