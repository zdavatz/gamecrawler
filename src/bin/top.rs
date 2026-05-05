//! Multi-designer top-N PDF.
//!
//! Reads from one or more cache directories produced by `gamecrawler`, picks
//! each designer's top-N games (by BGG overall rank, then users_rated as a
//! tie-breaker for unranked games), and renders one sectioned PDF.
//!
//! Usage:
//!   top --designer "Reiner Knizia:cache" \
//!       --designer "Wolfgang Kramer:cache-kramer" \
//!       --designer "Alan R. Moon:cache-moon" \
//!       --top 10 -o pdf/top-designers.pdf
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;

use gamecrawler::bga_match::BgaIndex;
use gamecrawler::bgg::{self, Game, LinkedItem, ThingDetail};
use gamecrawler::cache::Cache;
use gamecrawler::render;

#[derive(Parser, Debug)]
#[command(about = "Pick each designer's top-N games and render a combined PDF")]
struct Cli {
    /// One per designer: "Display Name:cache-dir". Repeat the flag.
    #[arg(long, required = true)]
    designer: Vec<String>,
    /// Top N per designer.
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// Output PDF.
    #[arg(long, short, default_value = "pdf/top-designers.pdf")]
    output: PathBuf,
    /// Title at the top of the PDF.
    #[arg(long, default_value = "Top games by designer")]
    title: String,
    /// Keep the intermediate HTML next to the PDF.
    #[arg(long)]
    keep_html: bool,
    /// Tag games playable on Board Game Arena. Path to a TSV produced by
    /// `bga_index`. Set to "" to disable.
    #[arg(long, default_value = "cache/bga-games.tsv")]
    bga_index: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let bga = if cli.bga_index.is_empty() {
        None
    } else {
        let p = std::path::Path::new(&cli.bga_index);
        if !p.exists() {
            eprintln!("[bga] no catalog at {} — skipping badge", p.display());
            None
        } else {
            let idx = BgaIndex::load(p)?;
            eprintln!("[bga] catalog: {} games", idx.len());
            Some(idx)
        }
    };
    let mut sections: Vec<(String, Vec<Game>)> = Vec::with_capacity(cli.designer.len());

    for entry in &cli.designer {
        let (name, dir) = entry
            .split_once(':')
            .ok_or_else(|| anyhow!("expected 'Name:cache-dir', got {entry:?}"))?;
        let cache = Cache::new(dir).with_context(|| format!("open cache {dir}"))?;
        let list_path = cache.list_path();
        let linked: Vec<LinkedItem> = Cache::load(&list_path)?
            .ok_or_else(|| anyhow!("no list.json at {} — run `gamecrawler` for {name} first", list_path.display()))?;

        let mut games: Vec<Game> = linked.into_iter().map(Game::from_linked).collect();

        // Pull in cached player/time supplements when available.
        for g in games.iter_mut() {
            let p = cache.item_path(&g.id);
            if let Some(d) = Cache::load::<ThingDetail>(&p)? {
                g.merge_detail(d);
            }
        }

        // Sort: ranked first by rank asc, then by users_rated desc.
        bgg::sort_games(&mut games);
        games.truncate(cli.top);

        let mut bga_hits = 0;
        if let Some(idx) = &bga {
            for g in &mut games {
                if let Some((slug, dname)) = idx.lookup(&g.name) {
                    g.bga_url = Some(format!("https://boardgamearena.com/gamepanel?game={slug}"));
                    if dname != g.name {
                        g.bga_name = Some(dname.to_string());
                    }
                    bga_hits += 1;
                }
            }
        }
        eprintln!(
            "[{name}] picked {} games · {bga_hits} on BGA",
            games.len()
        );
        sections.push((name.to_string(), games));
    }

    let html = render::build_sectioned_html(&cli.title, &sections);
    let html_path = cli.output.with_extension("html");
    std::fs::write(&html_path, &html)
        .with_context(|| format!("write {}", html_path.display()))?;

    eprintln!("[render] launching headless Chrome…");
    render::render_pdf(&html_path, &cli.output).await?;

    if !cli.keep_html {
        std::fs::remove_file(&html_path).ok();
    }
    let total: usize = sections.iter().map(|(_, g)| g.len()).sum();
    eprintln!("[render] wrote {} ({total} games)", cli.output.display());
    Ok(())
}
