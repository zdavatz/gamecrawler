use anyhow::{Context, Result};
use clap::Parser;
use futures::stream::StreamExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gamecrawler::bga_match::BgaIndex;
use gamecrawler::bgg::{self, Game, LinkedItem, ThingDetail};
use gamecrawler::cache::Cache;
use gamecrawler::render;

#[derive(Parser, Debug)]
#[command(about = "Crawl BGG for a designer's catalog and render it as PDF")]
struct Cli {
    /// BGG designer id (default: Reiner Knizia = 2).
    #[arg(long, default_value_t = 2)]
    designer_id: u32,
    /// Display name for the cover header.
    #[arg(long, default_value = "Reiner Knizia")]
    designer_name: String,
    /// Output PDF.
    #[arg(long, short, default_value = "pdf/knizia-games.pdf")]
    output: PathBuf,
    /// Cache directory.
    #[arg(long, default_value = "cache")]
    cache_dir: PathBuf,
    /// Concurrent BGG fetches for the per-game player/time supplement.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// Re-fetch even if cached.
    #[arg(long)]
    refresh: bool,
    /// Skip the per-game player/time/age supplement (faster; loses those columns).
    #[arg(long)]
    no_enrich: bool,
    /// Cap to N games (testing). 0 = all.
    #[arg(long, default_value_t = 0)]
    limit: usize,
    /// Keep the intermediate HTML next to the PDF.
    #[arg(long)]
    keep_html: bool,
    /// Tag games playable on BGA. Path to TSV from `bga_index`; "" disables.
    #[arg(long, default_value = "cache/bga-games.tsv")]
    bga_index: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache = Cache::new(&cli.cache_dir)?;
    let http = bgg::http_client()?;

    // 1. Designer credit list — paginated, already includes rating/rank/year/image.
    let list_path = cache.list_path();
    let linked: Vec<LinkedItem> = if !cli.refresh {
        match Cache::load(&list_path)? {
            Some(v) => {
                eprintln!("[list] using cache ({})", list_path.display());
                v
            }
            None => fetch_and_cache_list(&http, &cli, &list_path).await?,
        }
    } else {
        fetch_and_cache_list(&http, &cli, &list_path).await?
    };

    let mut linked = linked;
    if cli.limit > 0 {
        linked.truncate(cli.limit);
    }
    eprintln!("[list] {} unique games", linked.len());

    let mut games: Vec<Game> = linked.into_iter().map(Game::from_linked).collect();

    // 2. Optionally enrich with player count / playtime / age.
    if !cli.no_enrich {
        let total = games.len();
        let done = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(cache);
        let http = Arc::new(http);

        let ids: Vec<(usize, String)> = games
            .iter()
            .enumerate()
            .map(|(i, g)| (i, g.id.clone()))
            .collect();

        let details: Vec<(usize, Option<ThingDetail>)> = futures::stream::iter(
            ids.into_iter().map(|(idx, id)| {
                let http = http.clone();
                let cache = cache.clone();
                let done = done.clone();
                let refresh = cli.refresh;
                async move {
                    let r = fetch_or_load_detail(&http, &cache, &id, refresh)
                        .await
                        .map_err(|e| {
                            eprintln!("[enrich] {id}: {e:#}");
                            e
                        })
                        .ok();
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n % 25 == 0 || n == total {
                        eprintln!("[enrich] {n}/{total}");
                    }
                    (idx, r)
                }
            }),
        )
        .buffer_unordered(cli.concurrency)
        .collect()
        .await;

        for (idx, det) in details {
            if let Some(d) = det {
                games[idx].merge_detail(d);
            }
        }
    }

    bgg::sort_games(&mut games);

    // 2b. Optionally tag BGA-playable games.
    if !cli.bga_index.is_empty() {
        let p = std::path::Path::new(&cli.bga_index);
        if p.exists() {
            let idx = BgaIndex::load(p)?;
            let mut hits = 0;
            for g in &mut games {
                if let Some((slug, dname)) = idx.lookup(&g.name) {
                    g.bga_url = Some(format!("https://boardgamearena.com/gamepanel?game={slug}"));
                    if dname != g.name {
                        g.bga_name = Some(dname.to_string());
                    }
                    hits += 1;
                }
            }
            eprintln!("[bga] tagged {hits} of {} games as playable on BGA", games.len());
        }
    }

    // 3. Render.
    let html = render::build_html(&cli.designer_name, &games);
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

async fn fetch_and_cache_list(
    http: &reqwest::Client,
    cli: &Cli,
    path: &std::path::Path,
) -> Result<Vec<LinkedItem>> {
    eprintln!("[list] fetching designer {} from BGG…", cli.designer_id);
    let credits = bgg::list_designer_games(http, cli.designer_id).await?;
    Cache::save(path, &credits)?;
    eprintln!("[list] cached {} entries to {}", credits.len(), path.display());
    Ok(credits)
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
