//! Match a game against the cached BGA catalog.
//!
//! `cache/bga-games.tsv` is built by the `bga_index` binary by scraping the
//! game_list JSON inlined in https://boardgamearena.com/gamelist. Here we
//! load it and expose a normalized lookup that handles a few known
//! German↔English title aliases.
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub struct BgaIndex {
    by_norm: HashMap<String, (String, String)>,
    by_slug: HashMap<String, (String, String)>,
    aliases: HashMap<&'static str, &'static str>,
}

impl BgaIndex {
    pub fn load(path: &Path) -> Result<Self> {
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
        let aliases: HashMap<&'static str, &'static str> = [
            ("diamant", "incangold"),
            ("take5", "sechsnimmt"),
        ]
        .into_iter()
        .collect();
        Ok(Self {
            by_norm,
            by_slug,
            aliases,
        })
    }

    pub fn len(&self) -> usize {
        self.by_slug.len()
    }

    /// Returns (slug, display_name) if the game is on BGA.
    pub fn lookup(&self, name: &str) -> Option<(&str, &str)> {
        let n = normalize(name);
        if let Some(slug) = self.aliases.get(n.as_str()).copied() {
            if let Some((s, d)) = self.by_slug.get(slug) {
                return Some((s.as_str(), d.as_str()));
            }
        }
        if let Some((s, d)) = self.by_norm.get(&n) {
            return Some((s.as_str(), d.as_str()));
        }
        if let Some((s, d)) = self.by_slug.get(n.as_str()) {
            return Some((s.as_str(), d.as_str()));
        }
        None
    }
}

pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase().replace('&', "and");
    lower.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}
