mod banner;
mod body;
mod cli;
mod compress;
mod config;
mod file;
#[cfg(feature = "highlight")]
mod highlight;
mod logging;
mod markdown;
mod pages;
mod path;
mod serve;
mod style;
#[cfg(test)]
mod testkit;

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::cli::Args;
use crate::config::Config;

/// Resolve `--host`/`--port` into an address, accepting names such as `localhost`.
fn resolve_addr(host: &str, port: u16) -> std::io::Result<SocketAddr> {
  (host, port).to_socket_addrs()?.next().ok_or_else(|| {
    std::io::Error::new(
      std::io::ErrorKind::InvalidInput,
      format!("could not resolve host `{host}`"),
    )
  })
}

/// What the positional argument turned out to be: the root directory, and —
/// when serv was pointed at a file — that file. A lone file is served at `/`
/// and nowhere else, with its own folder as the root so the pages serv draws
/// still have a path to show.
fn resolve_root(dir: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
  let target = dir
    .canonicalize()
    .map_err(|e| format!("cannot serve `{}`: {e}", dir.display()))?;

  if target.is_dir() {
    return Ok((target, None));
  }
  if !target.is_file() {
    return Err(format!(
      "`{}` is neither a file nor a directory",
      target.display()
    ));
  }

  let root = target
    .parent()
    .expect("a canonical file path has a parent")
    .to_path_buf();
  Ok((root, Some(target)))
}

/// `--spa` and `--not-found` both name a file *inside* the served directory.
/// Pointed at one file there is no such directory, so say so at startup rather
/// than run a server that quietly ignores half of what it was asked for.
fn refuse_directory_flags(args: &Args) -> Result<(), String> {
  let shown = args.dir.display();
  for (flag, given) in [
    ("--spa", args.spa.is_some()),
    ("--not-found", args.not_found.is_some()),
  ] {
    if given {
      return Err(format!(
        "`{flag}` needs a directory to serve, not the single file `{shown}`"
      ));
    }
  }
  Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args = Args::parse();
  logging::init(args.quiet);

  let (root, only) = resolve_root(&args.dir)?;
  if only.is_some() {
    refuse_directory_flags(&args)?;
  }

  let config = Arc::new(Config {
    spa: args.spa.map(|p| root.join(p)),
    not_found: args.not_found.map(|p| root.join(p)),
    ext: args.ext,
    markdown: args.markdown,
    only,
    root,
  });

  let addr = resolve_addr(&args.host, args.port)?;
  let listener = TcpListener::bind(addr)
    .await
    .map_err(|e| format!("cannot listen on {addr}: {e}"))?;

  banner::print(&config, addr, args.quiet);

  loop {
    let (stream, _) = listener.accept().await?;
    let config = Arc::clone(&config);
    tokio::spawn(async move {
      let service = service_fn(move |req| serve::handle(req, Arc::clone(&config)));
      if let Err(err) = http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
      {
        log::debug!("connection error: {err}");
      }
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn args(dir: &std::path::Path, extra: &[&str]) -> Args {
    use clap::Parser;
    let dir = dir.to_string_lossy().into_owned();
    Args::try_parse_from(
      ["serv", &dir]
        .into_iter()
        .chain(extra.iter().copied())
        .map(str::to_string),
    )
    .expect("parsed args")
  }

  #[test]
  fn a_directory_is_the_root_and_nothing_is_singled_out() {
    let dir = tempfile::tempdir().unwrap();
    let (root, only) = resolve_root(dir.path()).unwrap();

    assert_eq!(root, dir.path().canonicalize().unwrap());
    assert_eq!(only, None);
  }

  #[test]
  fn a_file_is_singled_out_with_its_folder_as_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("index.html");
    std::fs::write(&file, "hi").unwrap();

    let (root, only) = resolve_root(&file).unwrap();

    assert_eq!(root, dir.path().canonicalize().unwrap());
    assert_eq!(only, Some(file.canonicalize().unwrap()));
  }

  #[test]
  fn a_path_that_is_not_there_says_which_path() {
    let dir = tempfile::tempdir().unwrap();
    let err = resolve_root(&dir.path().join("gone.html")).unwrap_err();

    assert!(err.starts_with("cannot serve `"), "{err}");
    assert!(err.contains("gone.html"), "{err}");
  }

  #[test]
  fn the_directory_flags_are_refused_for_a_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("index.html");
    std::fs::write(&file, "hi").unwrap();

    let err = refuse_directory_flags(&args(&file, &["-s"])).unwrap_err();
    assert!(err.contains("`--spa`"), "{err}");
    assert!(err.contains("index.html"), "{err}");

    let err = refuse_directory_flags(&args(&file, &["-n", "404.html"])).unwrap_err();
    assert!(err.contains("`--not-found`"), "{err}");
  }

  #[test]
  fn the_flags_a_single_file_can_live_with_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "hi").unwrap();

    assert!(refuse_directory_flags(&args(&file, &["-m", "-q", "-e"])).is_ok());
    assert!(refuse_directory_flags(&args(&file, &[])).is_ok());
  }

  #[test]
  fn resolves_a_literal_address() {
    let addr = resolve_addr("127.0.0.1", 8010).unwrap();

    assert_eq!(addr.to_string(), "127.0.0.1:8010");
  }

  #[test]
  fn resolves_the_wildcard_address() {
    assert_eq!(resolve_addr("0.0.0.0", 3000).unwrap().port(), 3000);
  }

  #[test]
  fn resolves_a_name() {
    // `localhost` comes out of the hosts file, so this needs no network; which
    // family it resolves to is the machine's business, not serv's.
    let addr = resolve_addr("localhost", 8010).unwrap();

    assert!(addr.ip().is_loopback());
    assert_eq!(addr.port(), 8010);
  }

  #[test]
  fn refuses_a_host_that_is_not_an_address() {
    // A bracketed IPv6 literal is not what `(host, port)` expects — the host
    // half is a bare address or a name, so this fails without a lookup.
    assert!(resolve_addr("[::1]", 8010).is_err());
  }
}
