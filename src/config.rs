use std::path::PathBuf;

/// Everything the request handler needs, resolved once at startup.
#[derive(Debug)]
pub struct Config {
    /// Canonical directory being served.
    pub root: PathBuf,
    /// Single-page app entry point, when `--spa` is in play.
    pub spa: Option<PathBuf>,
    /// Require `.html` in URLs instead of stripping it.
    pub ext: bool,
    /// Page to serve for a miss, when `--not-found` is in play.
    pub not_found: Option<PathBuf>,
}
