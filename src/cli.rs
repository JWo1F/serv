use std::path::PathBuf;

use clap::{ArgAction, Parser};

/// A small, fast development server for static sites, single-page apps and markdown.
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

  /// Render markdown files as pages in the browser
  #[arg(short = 'm', long)]
  pub markdown: bool,

  /// Page to serve when nothing matches
  #[arg(short = 'n', long = "not-found", value_name = "FILE")]
  pub not_found: Option<PathBuf>,

  /// Print help
  #[arg(long, action = ArgAction::Help)]
  help: Option<bool>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use clap::CommandFactory;
  use clap::error::ErrorKind;

  fn parse(args: &[&str]) -> Args {
    Args::try_parse_from(std::iter::once("serv").chain(args.iter().copied())).unwrap()
  }

  fn error(args: &[&str]) -> ErrorKind {
    Args::try_parse_from(std::iter::once("serv").chain(args.iter().copied()))
      .unwrap_err()
      .kind()
  }

  #[test]
  fn the_command_definition_is_sound() {
    // clap's own consistency check: duplicate shorts, bad defaults, an
    // unreachable argument. It panics rather than returning, so reaching the
    // end of this test is the assertion.
    Args::command().debug_assert();
  }

  #[test]
  fn serves_the_current_directory_on_the_loopback_by_default() {
    let args = parse(&[]);

    assert_eq!(args.dir, PathBuf::from("."));
    assert_eq!(args.host, "127.0.0.1");
    assert_eq!(args.port, 8010);
    assert_eq!(args.spa, None);
    assert_eq!(args.not_found, None);
    assert!(!args.quiet);
    assert!(!args.ext);
  }

  #[test]
  fn takes_the_directory_as_a_positional() {
    assert_eq!(parse(&["./dist"]).dir, PathBuf::from("./dist"));
    assert_eq!(parse(&["/var/www"]).dir, PathBuf::from("/var/www"));
  }

  #[test]
  fn refuses_a_second_positional() {
    assert_eq!(error(&["a", "b"]), ErrorKind::UnknownArgument);
  }

  #[test]
  fn short_h_is_the_host_not_help() {
    // The whole reason `disable_help_flag` is set: a dev server is asked to
    // listen on an address far more often than it is asked to explain itself.
    assert_eq!(parse(&["-h", "0.0.0.0"]).host, "0.0.0.0");
    assert_eq!(parse(&["--host", "localhost"]).host, "localhost");
  }

  #[test]
  fn help_is_the_long_flag_only() {
    assert_eq!(error(&["--help"]), ErrorKind::DisplayHelp);
    assert_eq!(error(&["-V"]), ErrorKind::DisplayVersion);
    assert_eq!(error(&["--version"]), ErrorKind::DisplayVersion);
  }

  #[test]
  fn reads_a_port() {
    assert_eq!(parse(&["-p", "8080"]).port, 8080);
    assert_eq!(parse(&["--port", "1"]).port, 1);
    assert_eq!(parse(&["-p", "65535"]).port, 65535);
  }

  #[test]
  fn refuses_a_port_that_is_not_a_port() {
    assert_eq!(error(&["-p", "http"]), ErrorKind::ValueValidation);
    assert_eq!(error(&["-p", "65536"]), ErrorKind::ValueValidation);
    assert_eq!(error(&["-p", "-1"]), ErrorKind::UnknownArgument);
  }

  #[test]
  fn spa_without_a_value_falls_back_to_index_html() {
    assert_eq!(parse(&["-s"]).spa, Some(PathBuf::from("index.html")));
    assert_eq!(parse(&["--spa"]).spa, Some(PathBuf::from("index.html")));
  }

  #[test]
  fn spa_takes_a_shell_of_its_own() {
    assert_eq!(
      parse(&["-s", "app.html"]).spa,
      Some(PathBuf::from("app.html"))
    );
    assert_eq!(
      parse(&["--spa=shell.html"]).spa,
      Some(PathBuf::from("shell.html"))
    );
  }

  #[test]
  fn an_optional_value_swallows_the_directory_that_follows_it() {
    // `num_args = 0..=1` cannot tell a shell from a positional, so `serv -s
    // ./dist` reads ./dist as the SPA file and serves the current directory.
    // Writing the directory first — `serv ./dist -s` — is unambiguous.
    let greedy = parse(&["-s", "./dist"]);
    assert_eq!(greedy.spa, Some(PathBuf::from("./dist")));
    assert_eq!(greedy.dir, PathBuf::from("."));

    let clear = parse(&["./dist", "-s"]);
    assert_eq!(clear.spa, Some(PathBuf::from("index.html")));
    assert_eq!(clear.dir, PathBuf::from("./dist"));
  }

  #[test]
  fn a_flag_after_a_bare_spa_is_still_a_flag() {
    let args = parse(&["-s", "-q"]);

    assert_eq!(args.spa, Some(PathBuf::from("index.html")));
    assert!(args.quiet);
  }

  #[test]
  fn reads_a_not_found_page() {
    assert_eq!(
      parse(&["-n", "404.html"]).not_found,
      Some(PathBuf::from("404.html"))
    );
    assert_eq!(
      parse(&["--not-found", "miss.html"]).not_found,
      Some(PathBuf::from("miss.html"))
    );
  }

  #[test]
  fn not_found_needs_a_value() {
    assert_eq!(error(&["-n"]), ErrorKind::InvalidValue);
  }

  #[test]
  fn reads_the_switches() {
    assert!(parse(&["-q"]).quiet);
    assert!(parse(&["--quiet"]).quiet);
    assert!(parse(&["-e"]).ext);
    assert!(parse(&["--ext"]).ext);
  }

  #[test]
  fn takes_short_switches_grouped() {
    let args = parse(&["-qe"]);

    assert!(args.quiet);
    assert!(args.ext);
  }

  #[test]
  fn takes_everything_at_once() {
    let args = parse(&[
      "./dist", "-h", "0.0.0.0", "-p", "3000", "-s", "app.html", "-n", "404.html", "-q", "-e",
    ]);

    assert_eq!(args.dir, PathBuf::from("./dist"));
    assert_eq!(args.host, "0.0.0.0");
    assert_eq!(args.port, 3000);
    assert_eq!(args.spa, Some(PathBuf::from("app.html")));
    assert_eq!(args.not_found, Some(PathBuf::from("404.html")));
    assert!(args.quiet);
    assert!(args.ext);
  }

  #[test]
  fn refuses_an_argument_it_does_not_know() {
    assert_eq!(error(&["--gzip"]), ErrorKind::UnknownArgument);
  }
}
