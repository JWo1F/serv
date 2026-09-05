use std::path::PathBuf;

use clap::{ArgAction, Parser};

/// A small, fast development server for static sites and single-page apps.
#[derive(Debug, Parser)]
#[command(
  name = "serv",
  version,
  about,
  disable_help_flag = true,
  after_help = "Examples:\n  \
        serv                          serve the current directory\n  \
        serv ./dist -p 8080           serve ./dist on port 8080\n  \
        serv -h 0.0.0.0 -p 1234       listen on every interface\n  \
        serv -s                       single-page app, falling back to index.html\n  \
        serv -s app.html -q           single-page app, no request logs"
)]
pub struct Args {
  /// Directory to serve
  #[arg(value_name = "DIR", default_value = ".")]
  pub dir: PathBuf,

  /// Address to listen on
  #[arg(short = 'h', long, value_name = "HOST", default_value = "127.0.0.1")]
  pub host: String,

  /// Port to listen on
  #[arg(short, long, value_name = "PORT", default_value_t = 8010)]
  pub port: u16,

  /// Serve a single-page app: unmatched paths fall back to this file
  #[arg(
        short,
        long,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "index.html"
    )]
  pub spa: Option<PathBuf>,

  /// Do not log requests
  #[arg(short, long)]
  pub quiet: bool,

  /// Require the .html extension in URLs instead of stripping it
  #[arg(short = 'e', long)]
  pub ext: bool,

  /// Page to serve when nothing matches
  #[arg(short = 'n', long = "not-found", value_name = "FILE")]
  pub not_found: Option<PathBuf>,

  /// Print help
  #[arg(long, action = ArgAction::Help)]
  help: Option<bool>,
}
