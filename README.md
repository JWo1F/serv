<img src="assets/poster.png" alt="serv" width="732">

A small, fast development server for static sites and single-page apps.

Point it at a folder and open the URL. No config file, no project layout to
conform to, no network access — one binary that reads your files and hands them
to the browser.

```console
$ serv

  serv▌ 0.1.0

  ╭──────────────────────────╮
  │  http://127.0.0.1:8010/  │
  ╰──────────────────────────╯

  root         ~/site
  contents     3 folders · 8 files · 412 kB
  index        index.html
  not found    built-in page
  urls         clean · /about serves about.html
  encoding     brotli · gzip
  logs         on

  ctrl-c to stop

200 GET  /                                      2.4 kB  0.6 ms
200 GET  /app.css                               8.1 kB  0.3 ms
404 GET  /favicon.ico                           9.0 kB  0.2 ms
```

## Install

Grab a binary from the [latest release](https://github.com/JWo1F/serv/releases/latest) —
macOS on Apple Silicon or Intel, Linux on arm64 or x86_64. The Linux builds are
statically linked against musl, so they run on any distribution.

```bash
tar -xzf serv-0.1.0-aarch64-apple-darwin.tar.gz
mv serv-0.1.0-aarch64-apple-darwin/serv /usr/local/bin/
```

Or with cargo:

```bash
cargo install --git https://github.com/JWo1F/serv
```

Or build it yourself — Rust 1.88 or newer:

```bash
git clone https://github.com/JWo1F/serv && cd serv && cargo install --path .
```

## Usage

```
serv [OPTIONS] [DIR]
```

| Option | Meaning |
| --- | --- |
| `DIR` | Folder to serve. Defaults to the current directory. |
| `-h`, `--host <HOST>` | Address to listen on. Default `127.0.0.1`. Names such as `localhost` are resolved. |
| `-p`, `--port <PORT>` | Port to listen on. Default `8010`. |
| `-s`, `--spa [<FILE>]` | Serve a single-page app: unmatched routes fall back to `FILE`, which defaults to `index.html`. |
| `-e`, `--ext` | Require the `.html` extension in URLs instead of stripping it. |
| `-n`, `--not-found <FILE>` | Page to serve for a miss, instead of serv's own. |
| `-q`, `--quiet` | Do not log requests. |
| `--help` | Print help. |
| `-V`, `--version` | Print the version. |

`-h` is the host, so help is `--help` only.

```bash
serv                      # the current folder, on http://127.0.0.1:8010
serv ./dist -p 8080       # a build output, on another port
serv -h 0.0.0.0           # reachable from your phone on the same network
serv -s                   # a single-page app falling back to index.html
serv -s app.html -q       # a different shell, and no request logs
serv -n 404.html          # your own not-found page
serv -e                   # URLs keep their .html
```

## How it serves

**Clean URLs, by default.** `/about` serves `about.html`, and `/about.html`
redirects to `/about` so a page has one address rather than two. `/docs/`
serves `docs/index.html`. Pass `-e` to turn all of that off and serve paths
exactly as written.

**Directories.** A folder with an `index.html` serves it. A folder without one
gets a browsable index — every entry with its type, size and date, and folders
first. A request for a folder without a trailing slash is redirected to one, so
relative links inside the page resolve.

**Single-page apps.** With `-s`, anything that does not exist on disk answers
with the app shell — but only for navigations. A request for a missing script or
stylesheet stays a 404 instead of becoming HTML with the wrong content type,
which is the failure mode that costs an afternoon.

**Nothing is cached in the server.** Every request reads the file from disk, so
what you just saved is what gets sent. The one cache serv takes part in is the
browser's, and only through revalidation: an `ETag` derived from the file's size
and modification time, `Cache-Control: no-cache`, and a `304` when the browser's
copy still matches. Editing a `--not-found` page or an SPA shell shows up on the
next reload, the same as any other file.

**Range requests.** `206 Partial Content` is answered properly, so `<video>` and
`<audio>` scrubbing works. `If-Range` is honoured, so a resumed download that
finds a changed file gets the whole thing rather than a corrupt splice.

**Compression.** Text responses between 1 KiB and 8 MiB are compressed on the
fly with brotli, or gzip where brotli is not accepted; `q=0` is honoured as the
refusal it is. Larger files keep their streaming path and their range support.
Media, archives and images are sent as they are.

**Large files stream.** A response is read from disk in chunks as it is written
to the socket, so serving a 2 GB video costs a buffer, not 2 GB of memory.

## Safety

serv is a *development* server: it binds to `127.0.0.1` unless you tell it
otherwise, and it has no authentication, no TLS and no rate limiting. Do not put
it on the public internet.

Path traversal is refused on two levels. Request paths are resolved segment by
segment — `..` cannot climb above the served folder, and a separator smuggled
through percent-encoding (`%2f`, `%5c`) is rejected rather than decoded into
one. Then the resolved path is canonicalised and checked to still be inside the
root, which catches a symlink pointing out of it.

## Built with

[hyper](https://hyper.rs) for HTTP/1, [tokio](https://tokio.rs) for the runtime,
[clap](https://docs.rs/clap) for the command line, and
[damask](https://github.com/jwo1f/damask) for the two pages serv draws itself —
both compiled into the binary, with their stylesheet inlined, so they render with
the network unplugged.

## License

MIT. See [LICENSE](LICENSE).
