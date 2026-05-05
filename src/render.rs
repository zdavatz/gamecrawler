//! Build an HTML overview page and print it to PDF via headless Chrome.
use anyhow::{anyhow, Result};
use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::bgg::Game;

pub fn build_html(designer: &str, games: &[Game]) -> String {
    let mut s = String::new();
    s.push_str(r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>"#);
    s.push_str(&html_text(designer));
    s.push_str(r#" — Board Game Catalog</title>
<style>
@page { size: A4; margin: 14mm 12mm; }
* { box-sizing: border-box; }
body { font-family: -apple-system, "Helvetica Neue", Arial, sans-serif;
       color: #111; margin: 0; font-size: 11px; line-height: 1.3; }
header { margin-bottom: 14px; border-bottom: 2px solid #111; padding-bottom: 8px;
         page-break-after: avoid; }
h1 { margin: 0 0 4px; font-size: 22px; }
.sub { color: #555; font-size: 10px; }
.grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.card { border: 1px solid #ddd; border-radius: 5px; padding: 8px;
        display: flex; gap: 9px; break-inside: avoid; page-break-inside: avoid;
        min-height: 100px; }
.thumb { width: 90px; height: 90px; flex-shrink: 0; display: flex;
         align-items: center; justify-content: center;
         background: #f4f4f4; border-radius: 3px; overflow: hidden; }
.thumb img { max-width: 100%; max-height: 100%; object-fit: contain; }
.thumb .nope { color: #aaa; font-size: 9px; }
.meta { flex: 1; min-width: 0; }
.row1 { display: flex; align-items: baseline; gap: 6px; margin-bottom: 2px; }
.rank { font-size: 10px; color: #888; font-variant-numeric: tabular-nums; min-width: 36px; }
.name { font-weight: 600; font-size: 12px; line-height: 1.2;
        overflow-wrap: anywhere; flex: 1; }
.year { font-size: 10px; color: #777; }
.stats { display: flex; gap: 10px; margin-top: 4px; flex-wrap: wrap; }
.stat { font-size: 10px; color: #444; }
.stat b { color: #111; font-weight: 600; }
.rating-hi { color: #0a6; }
.rating-md { color: #b80; }
.rating-lo { color: #c33; }
.unrated { color: #aaa; font-style: italic; font-size: 10px; }
.tag { display: inline-block; font-size: 9px; padding: 1px 5px; margin-left: 4px;
       border-radius: 3px; background: #eee; color: #555; vertical-align: 1px; }
.url { font-size: 9px; color: #0366d6; word-break: break-all;
       text-decoration: none; margin-top: 3px; display: block; }
.bga { display: inline-block; font-size: 9px; padding: 1px 6px;
       border-radius: 3px; background: #1a7f37; color: #fff;
       text-decoration: none; margin-left: 4px; vertical-align: 1px;
       font-weight: 600; letter-spacing: .2px; }
.bga::before { content: "▶ "; }
</style>
</head>
<body>
<header>
"#);

    let n_with_rating = games.iter().filter(|g| g.rating.is_some()).count();
    s.push_str(&format!(
        r#"<h1>{} — Board Game Catalog</h1>
<div class="sub">{} games credited as designer · {} ranked on BGG · generated {} · source: api.geekdo.com</div>
"#,
        html_text(designer),
        games.len(),
        n_with_rating,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    s.push_str("</header>\n<div class=\"grid\">\n");
    for g in games {
        s.push_str(&render_card(g));
    }
    s.push_str("</div></body></html>\n");
    s
}

/// Render a multi-section catalog (e.g., several designers in one PDF).
/// Each section gets its own subheading and a 2-column card grid.
pub fn build_sectioned_html(title: &str, sections: &[(String, Vec<Game>)]) -> String {
    let mut s = String::new();
    s.push_str(r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>"#);
    s.push_str(&html_text(title));
    s.push_str(r#"</title>
<style>
@page { size: A4; margin: 14mm 12mm; }
* { box-sizing: border-box; }
body { font-family: -apple-system, "Helvetica Neue", Arial, sans-serif;
       color: #111; margin: 0; font-size: 11px; line-height: 1.3; }
header { margin-bottom: 14px; border-bottom: 2px solid #111; padding-bottom: 8px;
         page-break-after: avoid; }
h1 { margin: 0 0 4px; font-size: 22px; }
.sub { color: #555; font-size: 10px; }
.section { margin-top: 16px; break-before: auto; }
.section + .section { margin-top: 22px; }
.section-head { font-size: 16px; font-weight: 600; padding: 6px 0 4px;
                border-bottom: 1px solid #888; margin-bottom: 8px;
                page-break-after: avoid; break-after: avoid; }
.section-head .count { color: #666; font-weight: 400; font-size: 11px; margin-left: 6px; }
.grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.card { border: 1px solid #ddd; border-radius: 5px; padding: 8px;
        display: flex; gap: 9px; break-inside: avoid; page-break-inside: avoid;
        min-height: 100px; }
.thumb { width: 90px; height: 90px; flex-shrink: 0; display: flex;
         align-items: center; justify-content: center;
         background: #f4f4f4; border-radius: 3px; overflow: hidden; }
.thumb img { max-width: 100%; max-height: 100%; object-fit: contain; }
.thumb .nope { color: #aaa; font-size: 9px; }
.meta { flex: 1; min-width: 0; }
.row1 { display: flex; align-items: baseline; gap: 6px; margin-bottom: 2px; }
.rank { font-size: 10px; color: #888; font-variant-numeric: tabular-nums; min-width: 36px; }
.name { font-weight: 600; font-size: 12px; line-height: 1.2;
        overflow-wrap: anywhere; flex: 1; }
.year { font-size: 10px; color: #777; }
.stats { display: flex; gap: 10px; margin-top: 4px; flex-wrap: wrap; }
.stat { font-size: 10px; color: #444; }
.stat b { color: #111; font-weight: 600; }
.rating-hi { color: #0a6; }
.rating-md { color: #b80; }
.rating-lo { color: #c33; }
.unrated { color: #aaa; font-style: italic; font-size: 10px; }
.url { font-size: 9px; color: #0366d6; word-break: break-all;
       text-decoration: none; margin-top: 3px; display: block; }
.bga { display: inline-block; font-size: 9px; padding: 1px 6px;
       border-radius: 3px; background: #1a7f37; color: #fff;
       text-decoration: none; margin-left: 4px; vertical-align: 1px;
       font-weight: 600; letter-spacing: .2px; }
.bga::before { content: "▶ "; }
</style>
</head>
<body>
<header>
"#);
    let total: usize = sections.iter().map(|(_, gs)| gs.len()).sum();
    s.push_str(&format!(
        r#"<h1>{}</h1>
<div class="sub">{} games across {} designers · generated {} · source: api.geekdo.com</div>
"#,
        html_text(title),
        total,
        sections.len(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    s.push_str("</header>\n");

    for (head, games) in sections {
        s.push_str(&format!(
            r#"<section class="section"><div class="section-head">{}<span class="count">— top {}</span></div><div class="grid">
"#,
            html_text(head),
            games.len()
        ));
        for g in games {
            s.push_str(&render_card(g));
        }
        s.push_str("</div></section>\n");
    }

    s.push_str("</body></html>\n");
    s
}

fn render_card(g: &Game) -> String {
    let rank_label = match g.rank {
        Some(r) => format!("#{r}"),
        None => "—".into(),
    };
    let bga_badge = match (&g.bga_url, &g.bga_name) {
        (Some(url), Some(bga_name)) => format!(
            r#"<a class="bga" href="{}" title="Play on Board Game Arena ({})">BGA</a>"#,
            html_attr(url),
            html_attr(bga_name)
        ),
        (Some(url), None) => format!(
            r#"<a class="bga" href="{}" title="Play on Board Game Arena">BGA</a>"#,
            html_attr(url)
        ),
        _ => String::new(),
    };
    let img_html = match &g.image {
        Some(url) if !url.is_empty() => {
            format!(r#"<img src="{}" alt="" loading="eager">"#, html_attr(url))
        }
        _ => r#"<span class="nope">no image</span>"#.into(),
    };
    let year = g
        .year
        .map(|y| format!(r#"<span class="year">({y})</span>"#))
        .unwrap_or_default();
    let rating_html = match g.rating {
        Some(r) => {
            let cls = if r >= 7.0 {
                "rating-hi"
            } else if r >= 6.0 {
                "rating-md"
            } else {
                "rating-lo"
            };
            let users = g.users_rated.unwrap_or(0);
            format!(
                r#"<span class="stat"><b class="{cls}">★ {:.2}</b> · {} users</span>"#,
                r,
                fmt_thousands(users)
            )
        }
        None => r#"<span class="unrated">unrated</span>"#.into(),
    };
    let players = match (g.min_players, g.max_players) {
        (Some(a), Some(b)) if a == b => format!(r#"<span class="stat"><b>{a}</b> players</span>"#),
        (Some(a), Some(b)) => format!(r#"<span class="stat"><b>{a}–{b}</b> players</span>"#),
        _ => String::new(),
    };
    let time = match (g.min_playtime, g.max_playtime) {
        (Some(a), Some(b)) if a == b && a > 0 => format!(r#"<span class="stat"><b>{a}′</b></span>"#),
        (Some(a), Some(b)) if a > 0 || b > 0 => {
            format!(r#"<span class="stat"><b>{a}–{b}′</b></span>"#)
        }
        _ => String::new(),
    };
    let owned = g
        .owned
        .filter(|n| *n > 0)
        .map(|n| format!(r#"<span class="stat">{} owned</span>"#, fmt_thousands(n)))
        .unwrap_or_default();

    format!(
        r#"<div class="card">
  <div class="thumb">{img_html}</div>
  <div class="meta">
    <div class="row1">
      <span class="rank">{rank_label}</span>
      <span class="name">{name}{bga_badge}</span>
      {year}
    </div>
    <div class="stats">{rating_html}{players}{time}{owned}</div>
    <a class="url" href="{url}">{url_short}</a>
  </div>
</div>
"#,
        name = html_text(&g.name),
        url = html_attr(&g.href),
        url_short = html_text(&shorten(&g.href, 70)),
    )
}

pub async fn render_pdf(html_path: &Path, pdf_path: &Path) -> Result<()> {
    if let Some(parent) = pdf_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let profile = std::env::current_dir()?.join(".chrome-profile");
    std::fs::create_dir_all(&profile).ok();
    clean_stale_singletons(&profile);

    let mut builder = BrowserConfig::builder()
        .user_data_dir(&profile)
        .arg("--no-default-browser-check")
        .arg("--no-first-run")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars");
    if let Some(p) = find_chrome() {
        builder = builder.chrome_executable(p);
    }
    let cfg = builder.build().map_err(|e| anyhow!(e))?;

    let (browser, mut handler) = Browser::launch(cfg).await?;
    let drain = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            let _ = h;
        }
    });

    let result = render_inner(&browser, html_path, pdf_path).await;

    // Best-effort tear-down so the profile lock releases.
    let mut browser = browser;
    let _ = browser.close().await;
    let _ = browser.wait().await;
    drain.abort();
    let _ = drain.await;

    result
}

async fn render_inner(browser: &Browser, html_path: &Path, pdf_path: &Path) -> Result<()> {
    let page = browser.new_page("about:blank").await?;
    let file_url = format!("file://{}", html_path.canonicalize()?.display());
    page.goto(&file_url).await?;
    page.wait_for_navigation().await?;
    wait_for_images(&page).await.ok();

    let mut params = PrintToPdfParams::default();
    params.print_background = Some(true);
    params.prefer_css_page_size = Some(true);
    page.save_pdf(params, pdf_path).await?;
    page.close().await.ok();
    Ok(())
}

async fn wait_for_images(page: &chromiumoxide::Page) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let script = r#"(() => {
        const imgs = Array.from(document.images);
        const loaded = imgs.filter(i => i.complete).length;
        return [loaded, imgs.length];
    })()"#;
    loop {
        let res = page.evaluate(script).await?;
        if let Some(arr) = res.value().and_then(|v| v.as_array()) {
            if arr.len() == 2 {
                let loaded = arr[0].as_u64().unwrap_or(0);
                let total = arr[1].as_u64().unwrap_or(0);
                if total > 0 && loaded == total {
                    return Ok(());
                }
            }
        }
        if std::time::Instant::now() > deadline {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn clean_stale_singletons(profile: &Path) {
    for f in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let p = profile.join(f);
        if let Ok(meta) = std::fs::symlink_metadata(&p) {
            let age = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .unwrap_or(Duration::from_secs(0));
            if age > Duration::from_secs(30) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn find_chrome() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("CHROME") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }
    let candidates = [
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/opt/google/chrome/chrome",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}

fn fmt_thousands(n: u32) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}
