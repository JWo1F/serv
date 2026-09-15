<img src="assets/poster.png" alt="serv" width="732">

A small, fast development server for static sites, single-page apps and markdown.

Point it at a folder and open the URL. No config file, no project layout to
conform to, no network access — one binary that reads your files and hands them
to the browser.

```console
$ serv

  serv▌ 0.4.0

  ╭──────────────────────────╮
  │  http://127.0.0.1:8010/  │
  ╰──────────────────────────╯

  root         ~/site
  contents     3 folders · 8 files · 412.0 kB
  index        index.html
  not found    built-in page
  urls         clean · /about serves about.html
  encoding     gzip · text between 1 KiB and 8 MiB
  logs         on

  ctrl-c to stop

200 GET  /                                      2.4 kB  0.6 ms
200 GET  /app.css                               8.1 kB  0.3 ms
404 GET  /favicon.ico                           9.0 kB  0.2 ms
```

## Install

With Homebrew, on macOS or Linux:

```bash
brew install jwo1f/tap/serv
```

That pours a prebuilt binary — no Rust toolchain, nothing to compile. The tap
is added along the way, so `brew upgrade` follows releases from then on.

Or grab a binary from the [latest release](https://github.com/JWo1F/serv/releases/latest) —
macOS on Apple Silicon or Intel, Linux on arm64 or x86_64. The Linux builds are
statically linked against musl, so they run on any distribution.

```bash
tar -xzf serv-0.4.0-aarch64-apple-darwin.tar.gz
mv serv-0.4.0-aarch64-apple-darwin/serv /usr/local/bin/
```

Or with cargo:

```bash
cargo install --git https://github.com/JWo1F/serv
```

Or build it yourself — Rust 1.88 or newer:

```bash
git clone https://github.com/JWo1F/serv && cd serv && cargo install --path .
```

**Syntax highlighting is a build option.** Every binary above — the Homebrew
bottle and the release tarballs included — is built without it, because the
grammars cost several megabytes. Building from source with the `highlight`
feature turns it on:

```bash
cargo install --git https://github.com/JWo1F/serv --features highlight
```

## Usage

```
serv [OPTIONS] [PATH]
```

| Option | Meaning |
| --- | --- |
| `PATH` | Folder to serve, or a single file to serve at `/`. Defaults to the current directory. |
| `-h`, `--host <HOST>` | Address to listen on. Default `127.0.0.1`. Names such as `localhost` are resolved. |
| `-p`, `--port <PORT>` | Port to listen on. Default `8010`. |
| `-s`, `--spa [<FILE>]` | Serve a single-page app: unmatched routes fall back to `FILE`, which defaults to `index.html`. |
| `-e`, `--ext` | Require the `.html` extension in URLs instead of stripping it. |
| `-m`, `--markdown` | Render `.md` files as pages instead of handing them over as text. |
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
serv -m ./docs            # read a folder of markdown in the browser
serv -m README.md         # read one file, served at / and nowhere else
```

## How it serves

**Clean URLs, by default.** `/about` serves `about.html`, and `/about.html`
redirects to `/about` so a page has one address rather than two. `/docs/`
serves `docs/index.html`. Pass `-e` to turn all of that off and serve paths
exactly as written.

**A single file.** Point serv at a file rather than a folder and it serves
that file at `/`, and nothing else — every other path, including the file's own
name, gets the not-found page. Nothing around it is exposed: the folder it sits
in is never listed and its neighbours are never served — which includes the
document's own pictures. An `<img>` or `![...](...)` pointing at a neighbouring
file gets the not-found page like any other path, so a README with a poster in
it wants `serv -m .` rather than `serv -m README.md`: a folder with no
`index.html` opens its README anyway, with the images alongside it. `-m` still
decides whether a `.md` file is rendered as a page or handed over as text, so
`serv -m NOTES.md` reads a document in the browser while `serv NOTES.md` hands
over the markdown. `-s` and `-n` both name a file inside a served folder, so
serv refuses them here instead of ignoring them.

**Directories.** A folder with an `index.html` serves it. A folder without one
gets a browsable index — every entry with its type, size and date, and folders
first. A request for a folder without a trailing slash is redirected to one, so
relative links inside the page resolve.

**Single-page apps.** With `-s`, anything that does not exist on disk answers
with the app shell — but only for navigations. A request for a missing script or
stylesheet stays a 404 instead of becoming HTML with the wrong content type,
which is the failure mode that costs an afternoon.

**Markdown, with `-m`.** A `.md` file becomes a page rather than a download:
`/about` serves `about.md`, and `/about.md` redirects to `/about` so a document
gets the same single address a page does. A folder with no `index.html` shows
its `index.md`, or failing that its `README.md`, instead of a file listing — so
pointing serv at a project opens the project. HTML wins every collision, and
without the flag a `.md` file is served exactly as it is on disk. GitHub's
extensions are all on: tables, task lists, footnotes, strikethrough. Links are
pointed at the pages serv serves — `guide.md` becomes `guide` — and a link off
the machine opens in its own tab. Front matter is dropped rather than printed,
and every heading gets an anchor so the `#links` in a README land. Fenced code
is set in plain monospace unless highlighting was built in.

A ```` ```mermaid ```` fence is drawn as a diagram. This is the one thing in
serv that reaches the network: mermaid is a browser library with no Rust
equivalent, so a page that draws a diagram — and only such a page — loads it
from jsdelivr. Everything else still works with the cable out.

**Syntax highlighting, with `--features highlight`.** Off by default, and a
build-time choice rather than a flag: syntect carries a dump of TextMate
grammars and the regex engine to run them, several times the size of serv
itself. Built in, a fenced block that names a language it knows is marked up
server-side; a language it does not know stays plain monospace, and so does a
block that names nothing — a `mermaid` fence is a diagram either way. The
markup is class-based rather than inline colours, so code takes its palette from
the same stylesheet as the page and follows it into dark mode. With `-m`, the
startup banner says which build you are running — `code highlighted` or `code
plain`.

**Nothing is cached in the server.** Every request reads the file from disk, so
what you just saved is what gets sent. The one cache serv takes part in is the
browser's, and only through revalidation: an `ETag` derived from the file's size
and modification time, `Cache-Control: no-cache`, and a `304` when the browser's
copy still matches. Editing a `--not-found` page or an SPA shell shows up on the
next reload, the same as any other file.

**Range requests.** `206 Partial Content` is answered properly, so `<video>` and
`<audio>` scrubbing works. `If-Range` is honoured, so a resumed download that
finds a changed file gets the whole thing rather than a corrupt splice.

**Compression.** Text responses between 1 KiB and 8 MiB are gzipped on the fly,
and `q=0` is honoured as the refusal it is. Larger files keep their streaming
path and their range support. Media, archives and images are sent as they are.

**Large files stream.** A response is read from disk in chunks as it is written
to the socket, so serving a 2 GB video costs a buffer, not 2 GB of memory.

## Safety

serv is a *development* server: it binds to `127.0.0.1` unless you tell it
otherwise, and it has no authentication, no TLS and no rate limiting. Do not put
it on the public internet.

It makes no outbound request either, with one exception: a markdown page that
draws a mermaid diagram loads mermaid from a CDN. A document with no diagram
never asks.

Path traversal is refused on two levels. Request paths are resolved segment by
segment — `..` cannot climb above the served folder, and a separator smuggled
through percent-encoding (`%2f`, `%5c`) is rejected rather than decoded into
one. Then the resolved path is canonicalised and checked to still be inside the
root, which catches a symlink pointing out of it.

## Built with

[hyper](https://hyper.rs) for HTTP/1, [tokio](https://tokio.rs) for the runtime,
[clap](https://docs.rs/clap) for the command line, and
[damask](https://github.com/jwo1f/damask) for the three pages serv draws itself
— the directory listing, the not-found page and a rendered markdown document.
All are compiled into the binary, with their stylesheet inlined, so they render
with the network unplugged.

## License

MIT. See [LICENSE](LICENSE).
