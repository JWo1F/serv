mod body;
mod cli;
mod config;
mod file;
mod pages;
mod path;
mod serve;

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
    root,
  });

  let addr = resolve_addr(&args.host, args.port)?;
  let listener = TcpListener::bind(addr)
    .await
    .map_err(|e| format!("cannot listen on {addr}: {e}"))?;

  println!("serv {}", env!("CARGO_PKG_VERSION"));
  println!("  root  {}", config.root.display());
  println!("  url   http://{addr}/");

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
