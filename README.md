# gamecrawler

Generate PDF catalogs of board games from BoardGameGeek data, with optional
"playable on Board Game Arena" badges.

## What's in the box

Four binaries, all wrapping a shared library:

| Binary | What it does |
|--------|-------------|
| `gamecrawler` | Crawl every game credited to a BGG designer and render a full catalog PDF (one card per game). |
| `popular` | Render the top-N most popular BGG games published in a year window. |
| `top` | Pick each designer's top-N games from cached crawls and produce one sectioned PDF. |
| `bga_index` | Scrape the inlined `game_list` JSON from boardgamearena.com/gamelist. Run this before any binary that uses `--bga-index`. |
| `bga_playable` | Render only the BGG-top-N games that are playable on Board Game Arena. |

## Prerequisites

- Rust 1.75+ (`cargo`).
- Google Chrome / Chromium (any of `/usr/bin/google-chrome-stable`,
  `/usr/bin/chromium`, `/opt/google/chrome/chrome`, or set `$CHROME`). The
  PDF rendering pipeline drives headless Chrome via `chromiumoxide`.
- **FlareSolverr** on `localhost:8191` *only* if you use `popular` or `bga_index`,
  which fetch Cloudflare-protected pages. One-liner:
  `docker run -d --name flaresolverr -p 8191:8191 ghcr.io/flaresolverr/flaresolverr:latest`

The designer crawler (`gamecrawler`) does not need FlareSolverr — it talks
directly to `api.geekdo.com`.

## Quick start

```sh
cargo build --release

# Full catalog of every Reiner Knizia (BGG designer #2) game.
./target/release/gamecrawler -o pdf/knizia.pdf

# Other designers — find IDs at boardgamegeek.com/boardgamedesigner/<ID>/<slug>.
./target/release/gamecrawler --designer-id 7 --designer-name "Wolfgang Kramer" \
    --cache-dir cache-kramer -o pdf/kramer.pdf
./target/release/gamecrawler --designer-id 9 --designer-name "Alan R. Moon" \
    --cache-dir cache-moon -o pdf/moon.pdf

# Top 10 most-owned BGG games published in the last 10 years.
./target/release/popular --top 10 -o pdf/popular.pdf

# Combined top-10-each, sectioned.
./target/release/top \
    --designer "Alan R. Moon:cache-moon" \
    --designer "Wolfgang Kramer:cache-kramer" \
    --designer "Reiner Knizia:cache" \
    --top 10 -o pdf/top.pdf

# Refresh the BGA catalog (~1300 games), then tag any of the above with
# a green BGA badge on games that are playable online.
./target/release/bga_index
./target/release/top --designer "Reiner Knizia:cache" --top 10 \
    --bga-index cache/bga-games.tsv -o pdf/top-knizia.pdf
```

The first BGG crawl takes ~17 s for ~800 games (8-way concurrent fetches with
on-disk JSON caching). Repeat runs are near-instant because every BGG response
is cached under `<cache-dir>/`.

## How "popular" is defined

`popular` ranks by `numowned` on BGG (the cleanest ownership-based proxy for
"many people have it"). Pass `--metric rank` for BGG's overall Bayesian rank
or `--metric voters` for most-rated. The tool does not have access to retail
sales data — no public source of that exists.

## How BGA matching works

`bga_index` opens `boardgamearena.com/gamelist` in headless Chrome,
extracts the `game_list` JSON that BGA inlines into the page (1273 entries
at last check), and writes it to `cache/bga-games.tsv` as `slug<TAB>display_name`.

`bga_match.rs` normalizes names (lowercase, strip punctuation, `&`→`and`) and
keeps a small alias table for known German↔English title pairs (e.g.,
"Diamant" → "Incan Gold", "Take 5" → "6 nimmt!"). Variants like *Ticket to
Ride: Nordic Countries* deliberately do **not** match base "Ticket to Ride".

## Layout

```
src/
  lib.rs            # pub mod bga_match, bgg, cache, flaresolverr, render
  main.rs           # gamecrawler binary
  bgg.rs            # api.geekdo.com client + Game model
  cache.rs          # filesystem JSON cache
  flaresolverr.rs   # tiny FlareSolverr POST client
  render.rs         # HTML → PDF via chromiumoxide
  bga_match.rs      # BGA catalog lookup + alias table
  bin/
    popular.rs
    top.rs
    bga_index.rs
    bga_playable.rs
```

`pdf/` and `cache*/` are gitignored; the cache directories are reusable
across runs but contain only fetched BGG data, no source.

## License

GPL-3.0
