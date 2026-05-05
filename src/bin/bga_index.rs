//! Open boardgamearena.com/gamelist in headless Chrome, scroll until lazy
//! loading stops, and emit one TSV row per game: `<slug>\t<display_name>`.
use anyhow::{anyhow, Context, Result};
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://boardgamearena.com/gamelist?type=all".to_string());
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "cache/bga-games.tsv".into());

    let profile = std::env::current_dir()?.join(".chrome-profile");
    std::fs::create_dir_all(&profile).ok();
    clean_stale(&profile);

    let mut builder = BrowserConfig::builder()
        .user_data_dir(&profile)
        .arg("--no-default-browser-check")
        .arg("--no-first-run")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
              (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36");
    for p in [
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/opt/google/chrome/chrome",
    ] {
        if std::path::Path::new(p).exists() {
            builder = builder.chrome_executable(PathBuf::from(p));
            break;
        }
    }
    let cfg = builder.build().map_err(|e| anyhow!(e))?;

    let (mut browser, mut handler) = Browser::launch(cfg).await?;
    let drain = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            let _ = h;
        }
    });

    let result = run(&browser, &url, &out_path).await;

    let _ = browser.close().await;
    let _ = browser.wait().await;
    drain.abort();
    let _ = drain.await;
    result
}

async fn run(browser: &Browser, url: &str, out_path: &str) -> Result<()> {
    let page = browser.new_page("about:blank").await?;
    page.goto(url).await?;
    page.wait_for_navigation().await?;
    eprintln!("[bga] loaded {url}");

    // Wait for game cards to begin appearing.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Scroll repeatedly. Stop once the count of game anchors stops changing
    // for two consecutive iterations or after 60 attempts.
    let mut last_count = 0usize;
    let mut stable_rounds = 0;
    for i in 0..60 {
        page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let res = page
            .evaluate(
                r#"document.querySelectorAll('a[href*="/gamepanel?game="]').length"#,
            )
            .await?;
        let count = res.value().and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if i % 5 == 0 {
            eprintln!("[bga] scroll {i:2} → {count} anchors");
        }
        if count == last_count {
            stable_rounds += 1;
            if stable_rounds >= 4 {
                break;
            }
        } else {
            stable_rounds = 0;
        }
        last_count = count;
    }

    let script = r#"
    (() => {
        const out = [];
        const seen = new Set();
        document.querySelectorAll('a[href*="/gamepanel?game="]').forEach(a => {
            const m = a.getAttribute('href').match(/game=([a-z0-9_]+)/);
            if (!m) return;
            const slug = m[1];
            // Try to extract a clean display name from common BGA selectors,
            // falling back to the anchor text.
            const card = a.closest('[class*="game"]') || a;
            let name = '';
            const cand = card.querySelector('.game_box_name, .gamename, .displayed_name, .name, h2, h3');
            if (cand) name = cand.textContent.trim();
            if (!name) name = a.textContent.trim();
            // Drop double-counted entries (BGA wraps each card in two anchors).
            if (seen.has(slug)) return;
            seen.add(slug);
            out.push({slug, name});
        });
        return out;
    })()
    "#;
    let res = page.evaluate(script).await?;
    let entries: Vec<serde_json::Value> = res
        .value()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let mut tsv = String::new();
    let mut count = 0;
    for e in &entries {
        let slug = e.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        let name = e
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .replace('\t', " ");
        if !slug.is_empty() {
            tsv.push_str(slug);
            tsv.push('\t');
            tsv.push_str(&name);
            tsv.push('\n');
            count += 1;
        }
    }

    if let Some(parent) = std::path::Path::new(out_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(out_path, &tsv).with_context(|| format!("write {out_path}"))?;
    eprintln!("[bga] wrote {count} games → {out_path}");
    Ok(())
}

fn clean_stale(profile: &std::path::Path) {
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
