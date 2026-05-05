//! Render a PDF of "top games by designer that are playable on Board Game Arena".
//!
//! Inputs (already on disk from prior runs):
//!   cache/list.json          — Knizia
//!   cache-kramer/list.json   — Kramer
//!   cache-moon/list.json     — Moon
//!   cache/bga-games.tsv      — full BGA catalog (slug \t display_name)
//!
//! For each designer we take their BGG-ranked top-N, filter to entries that
//! match a BGA catalog entry (by normalized name, with a small alias map for
//! known German↔English title pairs), and render via `build_sectioned_html`.
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

use gamecrawler::bgg::{self, Game, LinkedItem, ThingDetail};
use gamecrawler::cache::Cache;
use gamecrawler::render;

#[derive(Parser, Debug)]
struct Cli {
    /// Repeat: "Display Name:cache-dir".
    #[arg(long, required = true)]
    designer: Vec<String>,
    /// Take this many top BGG-ranked picks per designer before filtering to BGA.
    #[arg(long, default_value_t = 10)]
    pool: usize,
    /// BGA catalog TSV (slug \t display_name).
    #[arg(long, default_value = "cache/bga-games.tsv")]
    bga_index: PathBuf,
    /// Output PDF.
    #[arg(long, short, default_value = "pdf/bga-playable.pdf")]
    output: PathBuf,
    /// Title at the top of the PDF.
    #[arg(long, default_value = "Top games by designer — playable on Board Game Arena")]
    title: String,
    /// Keep the intermediate HTML next to the PDF.
    #[arg(long)]
    keep_html: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let bga = load_bga_index(&cli.bga_index)
        .with_context(|| format!("read {}", cli.bga_index.display()))?;
    eprintln!("[bga] catalog: {} games", bga.by_norm.len());

    let aliases: HashMap<&str, &str> = [
        ("diamant", "incangold"),
        ("take5", "sechsnimmt"),
    ]
    .into_iter()
    .collect();

    let mut sections: Vec<(String, Vec<Game>)> = Vec::new();

    for entry in &cli.designer {
        let (name, dir) = entry
            .split_once(':')
            .ok_or_else(|| anyhow!("expected 'Name:cache-dir', got {entry:?}"))?;
        let cache = Cache::new(dir).with_context(|| format!("open cache {dir}"))?;
        let list_path = cache.list_path();
        let linked: Vec<LinkedItem> = Cache::load(&list_path)?
            .ok_or_else(|| anyhow!("no list at {} — run gamecrawler first", list_path.display()))?;

        let mut games: Vec<Game> = linked.into_iter().map(Game::from_linked).collect();
        for g in &mut games {
            if let Some(d) = Cache::load::<ThingDetail>(&cache.item_path(&g.id))? {
                g.merge_detail(d);
            }
        }
        bgg::sort_games(&mut games);

        let pool: Vec<Game> = games.into_iter().take(cli.pool).collect();
        let mut filtered: Vec<Game> = Vec::new();
        for mut g in pool {
            let n = normalize(&g.name);
            let aliased_slug = aliases.get(n.as_str()).copied();
            let hit = aliased_slug
                .and_then(|s| bga.by_slug.get(s))
                .or_else(|| bga.by_norm.get(&n))
                .or_else(|| bga.by_slug.get(n.as_str()));
            if let Some((slug, dname)) = hit {
                // Repoint href to BGA's gamepanel URL. If BGA's display name
                // differs from BGG's, append it parenthetically.
                g.href = format!("https://boardgamearena.com/gamepanel?game={slug}");
                if normalize(dname) != n {
                    g.name = format!("{} (BGA: {})", g.name, dname);
                }
                filtered.push(g);
            }
        }
        eprintln!(
            "[{name}] {} of top {} pool playable on BGA",
            filtered.len(),
            cli.pool
        );
        sections.push((name.to_string(), filtered));
    }

    let total: usize = sections.iter().map(|(_, g)| g.len()).sum();
    if total == 0 {
        return Err(anyhow!("no playable games found — did you mean a wider --pool?"));
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
    eprintln!("[render] wrote {} ({total} games)", cli.output.display());
    Ok(())
}

struct BgaIndex {
    by_norm: HashMap<String, (String, String)>, // norm(display_name) → (slug, display_name)
    by_slug: HashMap<String, (String, String)>, // slug → (slug, display_name)
}

fn load_bga_index(path: &PathBuf) -> Result<BgaIndex> {
    let raw = std::fs::read_to_string(path)?;
    let mut by_norm = HashMap::new();
    let mut by_slug = HashMap::new();
    for line in raw.lines() {
        let mut it = line.splitn(2, '\t');
        let slug = it.next().unwrap_or("").trim().to_string();
        let dname = it.next().unwrap_or("").trim().to_string();
        if slug.is_empty() {
            continue;
        }
        by_slug.insert(slug.clone(), (slug.clone(), dname.clone()));
        if !dname.is_empty() {
            by_norm.insert(normalize(&dname), (slug.clone(), dname));
        }
    }
    Ok(BgaIndex { by_norm, by_slug })
}

fn normalize(s: &str) -> String {
    let lower = s.to_lowercase().replace('&', "and");
    lower.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}
