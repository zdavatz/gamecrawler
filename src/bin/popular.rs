//! Popular-games crawler.
//!
//! Fetches BGG's `/browse/boardgame?sort=numowned&sortdir=desc` (via
//! FlareSolverr because the page sits behind Cloudflare), filters to games
//! published in the requested year window, and renders the top-N as PDF
//! using the same renderer the designer crawler uses.
//!
//! "Popular" here = highest BGG `numowned` count, which is the cleanest
//! ownership-based proxy. Pass `--metric rank` if you'd rather rank by
//! BGG's overall rank.
use anyhow::{anyhow, Context, Result};
use chrono::Datelike;
use clap::{Parser, ValueEnum};
use regex::Regex;
use std::path::PathBuf;

use gamecrawler::bgg::{self, Game, ThingDetail};
use gamecrawler::cache::Cache;
use gamecrawler::flaresolverr::FlareSolverr;
use gamecrawler::render;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Metric {
    /// Most-owned on BGG.
    Numowned,
    /// BGG overall rank (lower is better; ranked games only).
    Rank,
    /// Most user ratings.
    Voters,
}

impl Metric {
    fn browse_param(self) -> &'static str {
        match self {
            Metric::Numowned => "numowned",
            Metric::Rank => "rank",
            Metric::Voters => "numvoters",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Metric::Numowned => "most owned on BGG",
            Metric::Rank => "highest BGG rank",
            Metric::Voters => "most rated on BGG",
        }
    }
}

#[derive(Parser, Debug)]
#[command(about = "Render the top-N most popular BGG games published in a year window")]
struct Cli {
    /// How many games to include.
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// Earliest publication year (inclusive).
    #[arg(long, default_value_t = default_since())]
    since_year: u16,
    /// Latest publication year (inclusive). Defaults to current year.
    #[arg(long, default_value_t = current_year())]
    until_year: u16,
    /// Popularity metric.
    #[arg(long, value_enum, default_value_t = Metric::Numowned)]
    metric: Metric,
    /// Output PDF.
    #[arg(long, short, default_value = "pdf/popular-games.pdf")]
    output: PathBuf,
    /// Cache directory.
    #[arg(long, default_value = "cache")]
    cache_dir: PathBuf,
    /// Re-fetch even if cached.
    #[arg(long)]
    refresh: bool,
    /// Keep the intermediate HTML next to the PDF.
    #[arg(long)]
    keep_html: bool,
    /// Maximum browse pages to scan (each page = 100 games before year filter).
    #[arg(long, default_value_t = 5)]
    max_pages: u32,
    /// FlareSolverr endpoint (default: $FLARESOLVERR_ENDPOINT or http://localhost:8191/v1).
    #[arg(long)]
    flaresolverr: Option<String>,
}

fn current_year() -> u16 {
    chrono::Utc::now().year() as u16
}
fn default_since() -> u16 {
    // 10 calendar years ago, inclusive (current year - 10).
    current_year().saturating_sub(10)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache = Cache::new(&cli.cache_dir)?;
    let http = bgg::http_client()?;
    let fs = match cli.flaresolverr.as_deref() {
        Some(ep) => FlareSolverr::with_endpoint(ep),
        None => FlareSolverr::new(),
    }?;

    eprintln!(
        "[browse] scanning BGG by {} for games {}–{}",
        cli.metric.browse_param(),
        cli.since_year,
        cli.until_year
    );

    let mut picks: Vec<BrowseRow> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for page in 1..=cli.max_pages {
        let url = format!(
            "https://boardgamegeek.com/browse/boardgame/page/{page}\
             ?sort={}&sortdir=desc",
            cli.metric.browse_param()
        );
        let html = fetch_browse_page(&fs, &cache, &url, page, cli.refresh).await?;
        let rows = parse_browse_page(&html)?;
        eprintln!(
            "[browse] page {page}: {} rows ({} matched year filter so far)",
            rows.len(),
            picks.len()
        );
        for r in rows {
            if r.year < cli.since_year || r.year > cli.until_year {
                continue;
            }
            if seen.insert(r.id.clone()) {
                picks.push(r);
                if picks.len() >= cli.top {
                    break;
                }
            }
        }
        if picks.len() >= cli.top {
            break;
        }
    }

    if picks.is_empty() {
        return Err(anyhow!(
            "no games matched year filter {}-{} after {} pages",
            cli.since_year,
            cli.until_year,
            cli.max_pages
        ));
    }
    eprintln!("[browse] {} picks selected", picks.len());

    // Enrich with image + player count via the per-thing endpoint.
    let mut games: Vec<Game> = Vec::with_capacity(picks.len());
    for r in &picks {
        let detail = fetch_or_load_detail(&http, &cache, &r.id, cli.refresh).await?;
        let mut g = r.into_game();
        g.merge_detail(detail.clone());
        // The geekitems endpoint also has the canonical image URL.
        if let Some(img) = pull_image(&http, &cache, &r.id, cli.refresh).await? {
            g.image = Some(img);
        }
        games.push(g);
    }

    let header = format!(
        "Top {} board games — {} ({}–{})",
        games.len(),
        cli.metric.label(),
        cli.since_year,
        cli.until_year
    );
    let html = render::build_html(&header, &games);
    let html_path = cli.output.with_extension("html");
    std::fs::write(&html_path, &html)
        .with_context(|| format!("write {}", html_path.display()))?;

    eprintln!("[render] launching headless Chrome…");
    render::render_pdf(&html_path, &cli.output).await?;

    if !cli.keep_html {
        std::fs::remove_file(&html_path).ok();
    }
    eprintln!(
        "[render] wrote {} ({} games)",
        cli.output.display(),
        games.len()
    );
    Ok(())
}

async fn fetch_browse_page(
    fs: &FlareSolverr,
    cache: &Cache,
    url: &str,
    page: u32,
    refresh: bool,
) -> Result<String> {
    let path = cache
        .root()
        .join("browse")
        .join(format!("page-{page}.html"));
    std::fs::create_dir_all(path.parent().unwrap()).ok();
    if !refresh && path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        if !raw.is_empty() {
            return Ok(raw);
        }
    }
    let html = fs.get(url).await.with_context(|| format!("flaresolverr GET {url}"))?;
    std::fs::write(&path, &html)?;
    Ok(html)
}

async fn fetch_or_load_detail(
    http: &reqwest::Client,
    cache: &Cache,
    id: &str,
    refresh: bool,
) -> Result<ThingDetail> {
    let p = cache.item_path(id);
    if !refresh {
        if let Some(v) = Cache::load::<ThingDetail>(&p)? {
            return Ok(v);
        }
    }
    let v = bgg::fetch_thing_detail(http, id).await?;
    Cache::save(&p, &v)?;
    Ok(v)
}

/// Fetches a richer geekitems JSON for the image URL, then caches its raw
/// imageurl field separately so subsequent runs are instant.
async fn pull_image(
    http: &reqwest::Client,
    cache: &Cache,
    id: &str,
    refresh: bool,
) -> Result<Option<String>> {
    let path = cache.root().join("images").join(format!("{id}.txt"));
    std::fs::create_dir_all(path.parent().unwrap()).ok();
    if !refresh && path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        return Ok(Some(raw).filter(|s| !s.is_empty()));
    }
    let url = format!(
        "https://api.geekdo.com/api/geekitems?nosession=1&objectid={id}&objecttype=thing"
    );
    let v: serde_json::Value = http
        .get(&url)
        .header(reqwest::header::REFERER, "https://boardgamegeek.com/")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("decode geekitems image {id}"))?;
    let img = v
        .get("item")
        .and_then(|i| i.get("imageurl"))
        .and_then(|s| s.as_str())
        .map(String::from)
        .unwrap_or_default();
    std::fs::write(&path, &img)?;
    Ok(Some(img).filter(|s| !s.is_empty()))
}

#[derive(Debug)]
struct BrowseRow {
    id: String,
    name: String,
    year: u16,
    geek_rating: Option<f32>,
    avg_rating: Option<f32>,
    num_voters: Option<u32>,
    num_owned: Option<u32>,
    href: String,
}

impl BrowseRow {
    fn into_game(&self) -> Game {
        Game {
            id: self.id.clone(),
            name: self.name.clone(),
            href: self.href.clone(),
            year: Some(self.year),
            image: None,
            min_players: None,
            max_players: None,
            min_playtime: None,
            max_playtime: None,
            min_age: None,
            rating: self.avg_rating.or(self.geek_rating),
            users_rated: self.num_voters,
            rank: None,
            owned: self.num_owned,
            weight: None,
            short_description: None,
            bga_url: None,
            bga_name: None,
        }
    }
}

/// Parses the BGG browse table. The table layout is stable but verbose, so
/// rather than a brittle one-shot regex we walk row-anchor blocks and pull
/// the four trailing `collection_bggrating` cells.
fn parse_browse_page(html: &str) -> Result<Vec<BrowseRow>> {
    // Each row's primary anchor is unique enough to slice by.
    let row_re = Regex::new(
        r#"href="(/boardgame/(\d+)/[^"]+)" class="primary">([^<]+)</a>[\s\S]{0,300}?\((\d{4})\)"#,
    )?;
    let stat_re = Regex::new(r#"<td[^>]*class="collection_bggrating"[^>]*>\s*([^<\s][^<]*?)\s*</td>"#)?;
    let mut rows = Vec::new();

    let matches: Vec<_> = row_re.captures_iter(html).collect();
    for (i, cap) in matches.iter().enumerate() {
        let href = cap.get(1).unwrap().as_str().to_string();
        let id = cap.get(2).unwrap().as_str().to_string();
        let name = html_unescape(cap.get(3).unwrap().as_str());
        let year: u16 = cap.get(4).unwrap().as_str().parse().unwrap_or(0);

        // Slice the chunk between this row's match and the next, then pull
        // the four bggrating cells out of it.
        let row_end = matches
            .get(i + 1)
            .map(|c| c.get(0).unwrap().start())
            .unwrap_or(html.len());
        let chunk = &html[cap.get(0).unwrap().end()..row_end];
        let stats: Vec<&str> = stat_re
            .captures_iter(chunk)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        let parse_f = |s: &&str| s.parse::<f32>().ok().filter(|n| *n > 0.0);
        let parse_u = |s: &&str| s.parse::<u32>().ok().filter(|n| *n > 0);
        let geek_rating = stats.first().and_then(parse_f);
        let avg_rating = stats.get(1).and_then(parse_f);
        let num_voters = stats.get(2).and_then(parse_u);
        let num_owned = stats.get(3).and_then(parse_u);

        rows.push(BrowseRow {
            id,
            name,
            year,
            geek_rating,
            avg_rating,
            num_voters,
            num_owned,
            href: format!("https://boardgamegeek.com{}", href),
        });
    }
    Ok(rows)
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
