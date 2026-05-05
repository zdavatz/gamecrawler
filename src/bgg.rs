//! BGG / geekdo.com JSON client.
//!
//! BGG's documented XMLAPI2 now returns 401 to unauthenticated callers, so
//! we use the same internal JSON endpoints the SPA on boardgamegeek.com
//! consumes. Two endpoints suffice:
//!
//! 1. `geekitem/linkeditems` — paginated list of all things credited to a
//!    person under a given linktype. The response per item already carries
//!    name, year, rating, rank, owners, and image URLs, so this single
//!    endpoint covers the bulk of the report.
//! 2. `geekitems?objectid=ID&objecttype=thing` — supplements a thing with
//!    player count / playtime / age (fields not in the list response).
//!    Optional; runs in parallel with bounded concurrency.
//!
//! Note on a previous bug: the *plural* `geekitems` endpoint with
//! `linkdata_index=boardgamedesigner` silently ignores `pageid` and always
//! returns the first 100 items. That's why an earlier draft of this file
//! produced ~9× duplicates.
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE: &str = "https://api.geekdo.com/api";
const REFERER: &str = "https://boardgamegeek.com/";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) gamecrawler/0.1";

pub fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(Into::into)
}

/// Subset of the linkeditems item we care about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedItem {
    pub objectid: String,
    pub name: String,
    pub href: String,
    pub yearpublished: Option<String>,
    pub usersrated: Option<String>,
    pub average: Option<String>,
    pub avgweight: Option<String>,
    pub numowned: Option<String>,
    pub numwish: Option<String>,
    pub numwanting: Option<String>,
    pub rank: Option<String>,
    #[serde(default)]
    pub images: ImageSet,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageSet {
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub square200: Option<String>,
    #[serde(default)]
    pub previewthumb: Option<String>,
}

#[derive(Deserialize)]
struct LinkedEnvelope {
    config: LinkedConfig,
    items: Vec<LinkedItem>,
}
#[derive(Deserialize)]
struct LinkedConfig {
    numitems: u32,
}

const PAGE_SIZE: u32 = 100;

/// Fetch every thing credited to `designer_id` as a board-game designer.
/// Pagination yields ~50 items per page server-side; we ask for `showcount=100`
/// in case BGG raises the cap. Caller should de-duplicate by `objectid`.
pub async fn list_designer_games(
    http: &Client,
    designer_id: u32,
) -> Result<Vec<LinkedItem>> {
    let url = |page: u32| {
        format!(
            "{BASE}/geekitem/linkeditems?ajax=1&objectid={designer_id}\
             &objecttype=person&linkdata_index=boardgamedesigner\
             &pageid={page}&showcount={PAGE_SIZE}&sort=name\
             &subtype=boardgamedesigner"
        )
    };

    let first: LinkedEnvelope = http
        .get(url(1))
        .header(reqwest::header::REFERER, REFERER)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("decode linkeditems page 1")?;
    let total = first.config.numitems as usize;
    let per_page = first.items.len().max(1);
    let mut all = first.items;
    let pages = total.div_ceil(per_page);

    for page in 2..=pages as u32 {
        let env: LinkedEnvelope = http
            .get(url(page))
            .header(reqwest::header::REFERER, REFERER)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .with_context(|| format!("decode linkeditems page {page}"))?;
        if env.items.is_empty() {
            break;
        }
        all.extend(env.items);
    }

    // De-duplicate by objectid; keep first occurrence (sort=name keeps it stable).
    let mut seen = std::collections::HashSet::new();
    all.retain(|it| seen.insert(it.objectid.clone()));

    if all.len() != total {
        eprintln!(
            "[list] designer claims {total} credits, got {} unique after dedup",
            all.len()
        );
    }
    Ok(all)
}

/// Per-thing supplement (the only fields not already in the list response).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThingDetail {
    pub minplayers: Option<String>,
    pub maxplayers: Option<String>,
    pub minplaytime: Option<String>,
    pub maxplaytime: Option<String>,
    pub minage: Option<String>,
    pub short_description: Option<String>,
}

#[derive(Deserialize)]
struct ThingEnvelope {
    item: ThingDetail,
}

pub async fn fetch_thing_detail(http: &Client, id: &str) -> Result<ThingDetail> {
    let url = format!("{BASE}/geekitems?nosession=1&objectid={id}&objecttype=thing");
    let env: ThingEnvelope = http
        .get(&url)
        .header(reqwest::header::REFERER, REFERER)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("decode geekitems {id}"))?;
    Ok(env.item)
}

/// Combined per-game record we feed the renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub href: String,
    pub year: Option<u16>,
    pub image: Option<String>,
    pub min_players: Option<u16>,
    pub max_players: Option<u16>,
    pub min_playtime: Option<u16>,
    pub max_playtime: Option<u16>,
    pub min_age: Option<u16>,
    pub rating: Option<f32>,
    pub users_rated: Option<u32>,
    pub rank: Option<u32>,
    pub owned: Option<u32>,
    pub weight: Option<f32>,
    pub short_description: Option<String>,
    /// If the game is on Board Game Arena, the gamepanel URL. Set by callers
    /// that have loaded the BGA catalog; otherwise None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bga_url: Option<String>,
    /// BGA's display name when it differs from BGG's primary name (e.g.,
    /// "Diamant" → "Incan Gold", "Take 5" → "6 nimmt!").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bga_name: Option<String>,
}

impl Game {
    pub fn from_linked(it: LinkedItem) -> Self {
        let parse_u: fn(&Option<String>) -> Option<u32> =
            |s| s.as_deref().and_then(|x| x.parse().ok());
        let parse_f: fn(&Option<String>) -> Option<f32> =
            |s| s.as_deref().and_then(|x| x.parse().ok());

        // BGG returns "0" for unranked / unrated; treat as missing.
        let nonzero = |o: Option<u32>| o.filter(|n| *n > 0);
        let nonzero_f = |o: Option<f32>| o.filter(|n| *n > 0.0);

        let image = it
            .images
            .square200
            .or(it.images.previewthumb)
            .or(it.images.thumb);
        let href = if it.href.starts_with("http") {
            it.href.clone()
        } else {
            format!("https://boardgamegeek.com{}", it.href)
        };

        Self {
            id: it.objectid,
            name: it.name,
            href,
            year: parse_u(&it.yearpublished).map(|y| y as u16),
            image,
            min_players: None,
            max_players: None,
            min_playtime: None,
            max_playtime: None,
            min_age: None,
            rating: nonzero_f(parse_f(&it.average)),
            users_rated: nonzero(parse_u(&it.usersrated)),
            rank: nonzero(parse_u(&it.rank)),
            owned: parse_u(&it.numowned),
            weight: nonzero_f(parse_f(&it.avgweight)),
            short_description: None,
            bga_url: None,
            bga_name: None,
        }
    }

    pub fn merge_detail(&mut self, d: ThingDetail) {
        let parse = |s: &Option<String>| s.as_deref().and_then(|x| x.parse().ok());
        self.min_players = parse(&d.minplayers);
        self.max_players = parse(&d.maxplayers);
        self.min_playtime = parse(&d.minplaytime);
        self.max_playtime = parse(&d.maxplaytime);
        self.min_age = parse(&d.minage);
        self.short_description = d.short_description.filter(|s| !s.trim().is_empty());
    }
}

/// Sort: ranked games first (by rank ascending), then everything else by
/// users-rated descending. Unrated games tail-end.
pub fn sort_games(games: &mut [Game]) {
    games.sort_by(|a, b| {
        let key = |g: &Game| -> (u8, i64, i64) {
            match (g.rank, g.users_rated) {
                (Some(r), _) => (0, r as i64, 0),
                (None, Some(u)) => (1, -(u as i64), 0),
                (None, None) => (2, 0, -(g.year.unwrap_or(0) as i64)),
            }
        };
        key(a).cmp(&key(b))
    });
}
