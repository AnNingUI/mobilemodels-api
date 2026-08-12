//! 每日数据搜集（collect）—— 从合法公开一手来源抓取事实数据，输出为标准 JSON 输入格式。
//!
//! 来源 #1: Google Play 官方设备兼容列表
//!   https://storage.googleapis.com/play_public/supported_devices.html
//!   Google 公开发布的全部 Android 设备清单（品牌 / 市场名 / codename / 型号），
//!   纯事实数据，无抓取限制。这是 codename 覆盖最全的官方来源。
//!
//! 输出: 与 `build --source` 兼容的 JSON 数组（brands/google-play.json）。

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

const GOOGLE_PLAY_URL: &str = "https://storage.googleapis.com/play_public/supported_devices.html";

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Parse the Google supported-devices HTML table into (brand, marketing_name, codename, model).
fn parse_rows(html: &str) -> Vec<(String, String, String, String)> {
    let mut rows = Vec::new();
    for tr in html.split("<tr").skip(1) {
        let end = match tr.find("</tr>") {
            Some(e) => e,
            None => continue,
        };
        let body = &tr[..end];
        let mut cells = Vec::new();
        for td in body.split("<td").skip(1) {
            let e = match td.find("</td>") {
                Some(e) => e,
                None => continue,
            };
            cells.push(strip_tags(&td[..e]).trim().to_string());
        }
        if cells.len() < 4 {
            continue;
        }
        rows.push((cells[0].clone(), cells[1].clone(), cells[2].clone(), cells[3].clone()));
    }
    rows
}

/// Fetch + parse Google Play supported devices, write standard JSON input file.
/// Returns the number of unique devices written.
pub fn collect_google_play(out_path: &Path, limit: Option<usize>) -> Result<usize> {
    println!("fetching {GOOGLE_PLAY_URL} ...");
    let resp = reqwest::blocking::get(GOOGLE_PLAY_URL)
        .with_context(|| "GET google play supported devices")?
        .error_for_status()?;
    let html = resp.text()?;
    println!("downloaded {} bytes, parsing rows ...", html.len());

    let mut seen = HashSet::new();
    let mut devices: Vec<Value> = Vec::new();
    for (brand_raw, marketing, codename, model) in parse_rows(&html) {
        let codename = unescape(&codename).trim().to_string();
        if codename.is_empty() {
            continue;
        }
        let brand = unescape(&brand_raw).trim().to_string();
        let brand = if brand.is_empty() { "Unknown".to_string() } else { brand };
        let marketing = unescape(&marketing).trim().to_string();
        let model = unescape(&model).trim().to_string();
        let name = if !marketing.is_empty() { marketing } else { model.clone() };

        let key = (brand.clone(), codename.clone(), model.clone());
        if !seen.insert(key) {
            continue;
        }
        let models = if model.is_empty() {
            Vec::new()
        } else {
            vec![json!({ "ids": [model], "market_name": name })]
        };
        devices.push(json!({
            "brand": brand,
            "name": name,
            "codename": codename,
            "models": models,
        }));
        if let Some(l) = limit {
            if devices.len() >= l {
                break;
            }
        }
    }

    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec(&devices)?;
    std::fs::write(out_path, bytes)?;
    Ok(devices.len())
}
