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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args = Args::parse();
  logging::init(args.quiet);

  let root = args
    .dir
    .canonicalize()
    .map_err(|e| format!("cannot serve `{}`: {e}", args.dir.display()))?;
  if !root.is_dir() {
    return Err(format!("`{}` is not a directory", root.display()).into());
  }

  let config = Arc::new(Config {
    spa: args.spa.map(|p| root.join(p)),
    not_found: args.not_found.map(|p| root.join(p)),
    ext: args.ext,
    markdown: args.markdown,
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
