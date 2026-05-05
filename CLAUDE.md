# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & run

```sh
cargo build --release
./target/release/gamecrawler          # full Knizia catalog → pdf/knizia-games.pdf
./target/release/popular              # top 10 most-owned (last 10 years)
./target/release/top --designer ...   # multi-designer sectioned PDF
./target/release/bga_index            # refresh BGA catalog → cache/bga-games.tsv
./target/release/bga_playable ...     # BGA-playable subset only
```

`cargo test` runs the (small) unit suite in `src/bgg.rs`.

## Architecture

A single library crate (`src/lib.rs`) plus four bins (`src/main.rs` and
`src/bin/*.rs`). Modules:

- **`bgg.rs`** — `api.geekdo.com` JSON client and the `Game` model. Two endpoints:
  `geekitem/linkeditems` for the designer credit list (already includes year,
  rating, rank, image), and `geekitems?objectid=ID&objecttype=thing` for
  player-count / playtime supplements.
- **`cache.rs`** — filesystem JSON cache. `<root>/list.json`,
  `<root>/items/<id>.json`, `<root>/dynamic/<id>.json`. Re-runs hit cache;
  pass `--refresh` to invalidate.
- **`render.rs`** — builds two HTML layouts (`build_html` and
  `build_sectioned_html`) sharing a single `render_card` helper, then prints
  to PDF via `chromiumoxide` (headless Chrome). Card supports an optional
  green BGA badge when `Game.bga_url` is set.
- **`flaresolverr.rs`** — minimal POST client to a local FlareSolverr
  container. Used only for the BGA catalog scrape (Cloudflare-protected).
- **`bga_match.rs`** — load `cache/bga-games.tsv`, normalize names
  (`lowercase + strip non-alphanumeric + & → and`), look up with a tiny
  alias table for known German↔English title pairs.

Adding a new binary: drop a file in `src/bin/`, register it under `[[bin]]`
in `Cargo.toml`, and import from `gamecrawler::{bgg, cache, render, ...}`.
The `top` binary is the canonical example of consuming on-disk caches
without making any new network calls.

## API gotchas

**BGG XMLAPI2 (`boardgamegeek.com/xmlapi2/...`) returns 401 to
unauthenticated callers.** Don't try to use it. Use `api.geekdo.com/api/`
endpoints instead — they're unauth and serve the same data.

**`api.geekdo.com/api/geekitems` (plural) silently ignores `pageid` when
called with `linkdata_index=...`.** It always returns the first page. The
correct paginated endpoint is `/api/geekitem/linkeditems` (singular). An
earlier version of `bgg.rs` used the wrong one and produced ~9× duplicate
entries; comments in the file warn future readers.

**BGA's `/gamelist` page is a Dojo SPA** that lazy-renders only ~50 games on
first paint. Scrolling the headless browser does not pull the rest. The
real source is the `game_list` JSON array inlined into the page HTML at load
time — `bga_index` extracts it with a bracket-counting parser.

## Caches

- `cache/` — Knizia (default) plus `cache/bga-games.tsv` (BGA catalog).
- `cache-<designer>/` — one per designer. Pass `--cache-dir cache-<name>` to
  isolate.
- `pdf/` — output PDFs. The renderer `mkdir -p`s the parent automatically.

All cache and pdf paths are gitignored.

## Conventions

- Every binary defaults its output PDF to `pdf/<binary>-...pdf` so generated
  artifacts stay grouped.
- `Game.href` should always point to BGG. The BGA URL goes in
  `Game.bga_url`, not by replacing `href`. Exception: `bga_playable.rs`
  intentionally repoints `href` because that PDF is BGA-focused.
- New PDFs should reuse `render::render_card` rather than rolling their own
  card HTML — the BGA badge support and the styling live there.
