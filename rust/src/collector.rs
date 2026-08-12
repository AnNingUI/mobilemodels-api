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
const WIKI_API: &str = "https://en.wikipedia.org/w/api.php";

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

// ---------------------------------------------------------------------------
// Wikipedia（CC BY-SA，可商用）—— Apple / Huawei(HarmonyOS) / Honor
// 页面: List_of_iPhone_models / List_of_Huawei_phones / List_of_Honor_phones
// ---------------------------------------------------------------------------

/// Decode HTML entities (named common ones + numeric). Char-aware (UTF-8 safe).
fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut ent = String::new();
        let mut found = false;
        while let Some(&n) = chars.peek() {
            if n == ';' {
                chars.next();
                found = true;
                break;
            }
            ent.push(n);
            chars.next();
        }
        let decoded = if found {
            match ent.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "middot" => Some('·'),
                _ => {
                    if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = ent.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };
        match decoded {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(&ent);
                if found {
                    out.push(';');
                }
            }
        }
    }
    out
}

/// Parse MediaWiki HTML into tables: Vec<rows> of Vec<(is_header_cell, text)>.
pub fn parse_wiki_tables(html: &str) -> Vec<Vec<Vec<(bool, String)>>> {
    let mut tables = Vec::new();
    let mut rest = html;
    while let Some(ts) = rest.find("<table") {
        let te = match rest[ts..].find("</table>") {
            Some(e) => ts + e,
            None => break,
        };
        let table_html = &rest[ts..te];
        rest = &rest[te + 8..];

        // only wikitables (data tables)
        if !table_html.contains("wikitable") {
            continue;
        }
        let mut rows = Vec::new();
        let mut row_start = table_html.find("<tr");
        while let Some(rs) = row_start {
            let re = match table_html[rs..].find("</tr>") {
                Some(e) => rs + e,
                None => break,
            };
            let row_html = &table_html[rs..re];
            row_start = table_html[re + 5..].find("<tr").map(|p| re + 5 + p);

            let mut cells = Vec::new();
            let mut cell_start = 0;
            loop {
                // 取 <td / <th 中最早出现的位置（不能用 or_else——会跳过先出现的 <th）
                let td = row_html[cell_start..].find("<td");
                let th = row_html[cell_start..].find("<th");
                let cs = match (td, th) {
                    (Some(a), Some(b)) => Some(a.min(b) + cell_start),
                    (a, b) => a.or(b).map(|p| p + cell_start),
                };
                let Some(cs) = cs else { break };
                let tag_end = row_html[cs..]
                    .find('>')
                    .map(|p| cs + p + 1)
                    .unwrap_or(row_html.len());
                let is_th = row_html[cs..tag_end].starts_with("<th");
                // 结束标签同样取最早出现：th 单元格后面紧跟的 </td> 不应被误用
                let ce_td = row_html[tag_end..].find("</td>");
                let ce_th = row_html[tag_end..].find("</th>");
                let cell_end = match (ce_td, ce_th) {
                    (Some(a), Some(b)) => tag_end + a.min(b),
                    (a, b) => a.or(b).map(|p| tag_end + p).unwrap_or(row_html.len()),
                };
                let raw = &row_html[tag_end..cell_end];
                cells.push((is_th, unescape_html(&strip_tags(raw)).trim().to_string()));
                cell_start = cell_end + 5;
            }
            if !cells.is_empty() {
                rows.push(cells);
            }
        }
        tables.push(rows);
    }
    tables
}

fn col_index(header: &[(bool, String)], keys: &[&str]) -> Option<usize> {
    header.iter().position(|(_, h)| {
        let h = h.to_lowercase();
        keys.iter().any(|k| h.contains(k))
    })
}

/// Extract devices from parsed tables.
/// kind = "apple" (Model + Model number A-numbers)
/// kind = "huawei" (Model + optional Codename/Model number; else name as model)
pub fn extract_from_tables(
    tables: &[Vec<Vec<(bool, String)>>],
    brand: &str,
    kind: &str,
    limit: Option<usize>,
) -> Vec<Value> {
    let mut devices: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for table in tables {
        let Some(header_row) = table.first() else { continue };
        let name_col = col_index(header_row, &["model"]);
        let modelnum_col = col_index(header_row, &["model number", "model no", "version"]);
        let codename_col = col_index(header_row, &["codename", "code name"]);
        if name_col.is_none() && modelnum_col.is_none() {
            continue;
        }
        for row in table.iter().skip(1) {
            // skip section-header rows (single th cell spanning the table)
            if row.len() == 1 && row[0].0 {
                continue;
            }
            let cell = |i: Option<usize>| -> Option<&String> {
                i.and_then(|ix| row.get(ix))
                    .map(|c| &c.1)
                    .filter(|s| !s.is_empty())
            };
            let name = cell(name_col).or_else(|| cell(modelnum_col)).cloned();
            let Some(name) = name else { continue };

            let (ids, market): (Vec<String>, String) = match kind {
                "apple" => {
                    let raw = cell(modelnum_col).cloned().unwrap_or_else(|| name.clone());
                    // Apple model numbers look like A1332 / A2849
                    let mut ids: Vec<String> = raw
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                        .filter(|t| {
                            let t = t.trim();
                            t.len() >= 4
                                && t.starts_with('A')
                                && t[1..].chars().all(|c| c.is_ascii_digit())
                        })
                        .map(|t| t.to_string())
                        .collect();
                    ids.sort();
                    ids.dedup();
                    if ids.is_empty() {
                        // 无 A 编号的行（日期/版本说明等）不是设备，跳过
                        continue;
                    }
                    (ids, name.clone())
                }
                _ => {
                    let ids: Vec<String> = cell(modelnum_col)
                        .map(|raw| {
                            raw.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                                .filter(|t| t.len() >= 3)
                                .map(|t| t.to_string())
                                .collect()
                        })
                        // Huawei 的 Codename 列即型号（如 VOG-L29 / ALN-AL10）
                        .or_else(|| cell(codename_col).map(|c| vec![c.clone()]))
                        .unwrap_or_default();
                    (ids, name.clone())
                }
            };

            let key = (brand.to_string(), name.clone(), ids.join("|"));
            if !seen.insert(key) {
                continue;
            }
            let models = if ids.is_empty() {
                vec![json!({ "ids": [name], "market_name": name })]
            } else {
                vec![json!({ "ids": ids, "market_name": market })]
            };
            let codename = cell(codename_col).cloned().unwrap_or_default();
            devices.push(json!({
                "brand": brand,
                "name": name,
                "codename": codename,
                "models": models,
            }));
            if let Some(l) = limit {
                if devices.len() >= l {
                    return devices;
                }
            }
        }
    }
    devices
}

/// Fetch a Wikipedia page (MediaWiki API, HTML) and extract devices.
/// Note: Wikipedia is unreachable from mainland CN networks — run via GitHub
/// Actions (US runners) or a proxy. Falls back gracefully on failure.
pub fn collect_wikipedia(
    page: &str,
    brand: &str,
    kind: &str,
    out_path: &Path,
    limit: Option<usize>,
) -> Result<usize> {
    let url = format!("{WIKI_API}?action=parse&page={page}&prop=text&format=json&formatversion=2");
    println!("fetching {page} (Wikipedia) ...");
    // Wikipedia 政策：必须带描述性 User-Agent，否则 403
    let client = reqwest::blocking::Client::builder()
        .user_agent("mobilemodels-db/0.1 (device-model collector; https://github.com/)")
        .build()
        .context("build http client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("GET wikipedia page {page}"))?
        .error_for_status()?;
    let text = resp.text()?;
    let v: Value = serde_json::from_str(&text).context("wikipedia api json")?;
    let html = v["parse"]["text"]
        .as_str()
        .context("wikipedia api: no parse.text (page may not exist)")?;
    let tables = parse_wiki_tables(html);
    println!("  parsed {} wikitables", tables.len());
    let devices = match kind {
        "apple-columns" => {
            let mut d = extract_apple_columns(&tables);
            if let Some(l) = limit {
                d.truncate(l);
            }
            d
        }
        _ => extract_from_tables(&tables, brand, kind, limit),
    };
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out_path, serde_json::to_vec(&devices)?)?;
    Ok(devices.len())
}

// ---------------------------------------------------------------------------
// Apple 官方支持页（HT201296: Identify your iPhone model）
// 结构: <h2 class="gb-header">iPhone 15 Pro Max</h2> ... "Model numbers: A2849 (...), A3105 (...)"
// 官方事实数据；大陆网络可直连（无需代理）。
// ---------------------------------------------------------------------------

/// Earliest occurrence of either marker (byte index).
fn next_marker(s: &str) -> Option<usize> {
    let g = s.find("gb-header");
    let m = s.find("Model numbers:");
    match (g, m) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Parse the Apple support page HTML into (name, Vec<A-numbers>).
/// Single pass: track the last `<h2 class="gb-header">NAME</h2>`; whenever a
/// "Model numbers:" text block appears, attach its A-numbers to that name.
pub fn parse_apple_support(html: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut header: Option<String> = None;
    let mut seen = HashSet::new();
    let mut rest = html;
    while let Some(i) = next_marker(rest) {
        if rest[i..].starts_with("gb-header") {
            let seg = &rest[i..];
            let after = match seg.find('>') {
                Some(p) => p + 1,
                None => break,
            };
            let text_end = match seg[after..].find('<') {
                Some(p) => after + p,
                None => break,
            };
            let name = strip_tags(&seg[after..text_end]).trim().to_string();
            if !name.is_empty() {
                header = Some(name);
            }
            rest = &rest[i + 8..];
        } else {
            let seg = &rest[i..];
            let end = seg.find('<').unwrap_or(seg.len());
            let text = strip_tags(&seg[..end]);
            let mut ids: Vec<String> = text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|t| {
                    let t = t.trim();
                    t.len() == 5 && t.starts_with('A') && t[1..].chars().all(|c| c.is_ascii_digit())
                })
                .map(|t| t.to_string())
                .collect();
            ids.sort();
            ids.dedup();
            if let Some(name) = header.clone() {
                if !ids.is_empty() && name.contains("iPhone") {
                    let key = (name.clone(), ids.join("|"));
                    if seen.insert(key) {
                        out.push((name, ids));
                    }
                }
            }
            rest = &rest[i + 1..];
        }
    }
    out
}

/// Fetch Apple's official "Identify your iPhone model" page and write JSON.
pub fn collect_apple_support(out_path: &Path, limit: Option<usize>) -> Result<usize> {
    let url = "https://support.apple.com/en-us/HT201296";
    println!("fetching {url} ...");
    let client = reqwest::blocking::Client::builder()
        .user_agent("mobilemodels-db/0.1 (device-model collector; https://github.com/)")
        .build()
        .context("build http client")?;
    let resp = client
        .get(url)
        .send()
        .context("GET apple support page")?
        .error_for_status()?;
    let html = resp.text()?;
    let pairs = parse_apple_support(&html);
    let devices: Vec<Value> = pairs
        .into_iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|(name, ids)| {
            json!({
                "brand": "Apple",
                "name": name,
                "models": [{ "ids": ids, "market_name": name }],
            })
        })
        .collect();
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out_path, serde_json::to_vec(&devices)?)?;
    Ok(devices.len())
}

/// Extract the COMPLETE Apple iPhone list from the "List of iPhone models"
/// page — 4 transposed tables where devices are COLUMNS:
///   row0: Model | iPhone 17e | iPhone 17 Pro Max | ...
///   row2: Basic Info | Hardware Strings | iPhone18,5 | ...   (codename)
///   row3: Model number | A3575A3634A3635 | A3257... | ...    (A-numbers, 5-char chunks)
pub fn extract_apple_columns(tables: &[Vec<Vec<(bool, String)>>]) -> Vec<Value> {
    let mut devices = Vec::new();
    let mut seen = HashSet::new();
    for table in tables {
        let Some(row0) = table.first() else { continue };
        if row0.len() < 3 || row0[0].1.trim() != "Model" {
            continue;
        }
        let names: Vec<String> = row0[1..]
            .iter()
            .map(|c| c.1.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            continue;
        }
        let mut a_rows: Vec<Vec<String>> = Vec::new();
        let mut hw_row: Vec<String> = Vec::new();
        for row in table.iter().skip(1) {
            // 标签单元格可能在 col0（Model number）或 col1（rowgroup: Basic Info + Hardware Strings），
            // 取标签之后的所有单元格作为该行值
            let label_pos = row.iter().position(|c| {
                let t = c.1.to_lowercase();
                t.contains("model number") || t.contains("hardware strings") || t == "hardware"
            });
            if let Some(lp) = label_pos {
                let values: Vec<String> = row[lp + 1..]
                    .iter()
                    .map(|c| c.1.trim().to_string())
                    .collect();
                if row[lp].1.to_lowercase().contains("model number") {
                    a_rows.push(values);
                } else {
                    hw_row = values;
                }
            }
        }
        for (i, name) in names.into_iter().enumerate() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let mut ids: Vec<String> = a_rows
                .iter()
                .flat_map(|row| row.get(i).cloned().into_iter())
                .flat_map(|raw| {
                    // Apple A-numbers are exactly 5 chars (A + 4 digits),
                    // often concatenated: "A3575A3634A3635"
                    let compact: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                    let b = compact.as_bytes();
                    let mut out = Vec::new();
                    let mut j = 0;
                    while j + 5 <= b.len() {
                        let chunk = &compact[j..j + 5];
                        if chunk.starts_with('A') && chunk[1..].chars().all(|c| c.is_ascii_digit()) {
                            out.push(chunk.to_string());
                        }
                        j += 5;
                    }
                    out
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            ids.sort();
            if ids.is_empty() {
                continue;
            }
            let codename = hw_row.get(i).cloned().unwrap_or_default();
            devices.push(json!({
                "brand": "Apple",
                "name": name,
                "codename": codename,
                "models": [{ "ids": ids, "market_name": name }],
            }));
        }
    }
    devices
}

// ---------------------------------------------------------------------------
// 工信部电信设备进网许可（TENAA）—— 国行手机进网型号权威数据
// 新站点: https://jwxk.miit.gov.cn（旧 tenaa.com.cn 已停用）
// 接口: /dev-api-20/internetService/CertificateQuery
//   按设备名称子串查询（所有手机证书名均含"移动电话机"），分页遍历全量。
// 响应记录: applyOrg(生产企业) / equipmentModel(进网型号) / equipmentName(设备类别名)
//          / licenseNo(许可证编号) / regDate / endDate
// ---------------------------------------------------------------------------

const TENAA_API: &str =
    "https://jwxk.miit.gov.cn/dev-api-20/internetService/CertificateQuery";

/// 生产企业(法人名) → 品牌名 归一化（包含匹配，未命中保留原名）。
fn normalize_cn_brand(org: &str) -> String {
    let rules = [
        ("小米", "小米"),
        ("华为", "华为"),
        ("OPPO", "OPPO"),
        ("维沃", "vivo"),
        ("荣耀", "荣耀"),
        ("中兴", "中兴"),
        ("努比亚", "努比亚"),
        ("三星", "三星"),
        ("苹果", "Apple"),
        ("诺基亚", "诺基亚"),
        ("摩托罗拉", "摩托罗拉"),
        ("联想", "联想"),
        ("索尼", "索尼"),
        ("魅族", "魅族"),
        ("一加", "一加"),
        ("真我", "真我"),
        ("酷派", "酷派"),
        ("TCL", "TCL"),
        ("金立", "金立"),
        ("360", "360"),
    ];
    for (key, brand) in rules {
        if org.contains(key) {
            return brand.to_string();
        }
    }
    org.to_string()
}

/// Collect ALL Chinese phone network-access certificates (进网许可) by
/// **date-window recursion**: the API caps any single query at 30 records and
/// pagination beyond that fails, so we bisect the date range until each window
/// holds <= 30 records. ~44k records => ~1500 leaf queries.
pub fn collect_tenaa(out_path: &Path, max_devices: Option<usize>, delay_ms: u64) -> Result<usize> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("mobilemodels-db/0.1 (device-model collector; https://github.com/)")
        .build()
        .context("build http client")?;
    let mut devices: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    let mut queries = 0usize;
    let (start, end) = ("2000-01-01", "2026-12-31");

    tenaa_window(
        &client, start, end, &mut devices, &mut seen, delay_ms, max_devices, &mut queries, 0,
    )?;
    println!("  total queries: {queries}");
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out_path, serde_json::to_vec(&devices)?)?;
    Ok(devices.len())
}

fn tenaa_window(
    client: &reqwest::blocking::Client,
    start: &str,
    end: &str,
    devices: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    delay_ms: u64,
    max_devices: Option<usize>,
    queries: &mut usize,
    depth: usize,
) -> Result<()> {
    if max_devices.map(|m| devices.len() >= m).unwrap_or(false) {
        return Ok(());
    }
    if *queries >= 6000 || depth > 20 {
        // 防失控保护（服务端异常时）
        return Ok(());
    }
    *queries += 1;
    let url = format!(
        "{TENAA_API}?isphoto=0&pageNo=1&pageSize=30&equipmentName={}&startDate={start}&endDate={end}",
        percent_encode("移动电话机")
    );
    let mut text = String::new();
    let mut ok = false;
    for attempt in 0..3 {
        match client
            .get(&url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
        {
            Ok(t) => {
                text = t;
                ok = true;
                break;
            }
            Err(e) => {
                eprintln!("  window {start}~{end} attempt {}/3 error: {e}", attempt + 1);
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
        }
    }
    if !ok {
        return Ok(()); // give up this window, keep the rest
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if v["code"].as_i64().unwrap_or(0) != 200 || v["data"].is_null() {
        // "无该条纪录" = empty window
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        return Ok(());
    }
    let data = &v["data"];
    let records = data["records"].as_array().cloned().unwrap_or_default();

    // 服务端 total 同样被截断为 30；返回满 30 条即可能还有更多 → 按日期二分
    if records.len() >= 30 && start < end {
        let mid = mid_date(start, end);
        let next = add_days(&mid, 1);
        // 先处理较新的一半：服务端按注册序返回窗口内最早 30 条，
        // 左半（较老）会重复返回同批旧记录，先右后左可确保每片窗口拿到自己的数据
        tenaa_window(client, &next, end, devices, seen, delay_ms, max_devices, queries, depth + 1)?;
        tenaa_window(client, start, &mid, devices, seen, delay_ms, max_devices, queries, depth + 1)?;
        return Ok(());
    }
    // leaf: add all records (<= 30)
    if devices.len() % 500 < 30 {
        println!("  ... {} devices so far (window {start}~{end})", devices.len());
    }
    for r in &records {
        let model = r["equipmentModel"].as_str().unwrap_or("").trim().to_string();
        let cert_name = r["equipmentName"].as_str().unwrap_or("").trim().to_string();
        let org = r["applyOrg"].as_str().unwrap_or("").trim().to_string();
        if model.is_empty() || !seen.insert(model.clone()) {
            continue;
        }
        let brand = normalize_cn_brand(&org);
        let name = if cert_name.is_empty() { model.clone() } else { cert_name };
        devices.push(json!({
            "brand": brand,
            "name": name,
            "series": "进网许可",
            "code": r["licenseNo"].as_str().unwrap_or(""),
            "models": [{ "ids": [model], "market_name": name }],
        }));
        if max_devices.map(|m| devices.len() >= m).unwrap_or(false) {
            break;
        }
    }
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
    Ok(())
}

fn parse_date(s: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp() / 86400)
        .unwrap_or(0)
}

fn format_date(days: i64) -> String {
    chrono::DateTime::from_timestamp(days * 86400, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn mid_date(a: &str, b: &str) -> String {
    format_date((parse_date(a) + parse_date(b)) / 2)
}

fn add_days(date: &str, n: i64) -> String {
    format_date(parse_date(date) + n)
}

/// Minimal percent-encoding for query strings (UTF-8).
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE_FIXTURE: &str = r#"<table class="wikitable"><tbody>
<tr><th>Model</th><th>Release date</th><th>Model number</th><th>SoC</th></tr>
<tr><td>iPhone 3G</td><td>July 11, 2008</td><td>A1324</td><td>Samsung S5L8920</td></tr>
<tr><td>iPhone 4</td><td>June 24, 2010</td><td>A1332 (GSM), A1349 (CDMA)</td><td>Apple A4</td></tr>
<tr><th colspan="4">2011: iPhone 4S</th></tr>
<tr><td>iPhone 4S</td><td>October 14, 2011</td><td>A1431 (GSM), A1387 (CDMA)</td><td>Apple A5</td></tr>
</tbody></table>"#;

    #[test]
    fn parse_apple_fixture() {
        let tables = parse_wiki_tables(APPLE_FIXTURE);
        assert_eq!(tables.len(), 1);
        let devices = extract_from_tables(&tables, "Apple", "apple", None);
        assert_eq!(devices.len(), 3, "section header row must be skipped");
        let d0 = &devices[0];
        assert_eq!(d0["name"], "iPhone 3G");
        assert_eq!(d0["models"][0]["ids"][0], "A1324");
        let d1 = &devices[1];
        let ids: Vec<&str> = d1["models"][0]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["A1332", "A1349"], "split multiple A-numbers");
        let d3 = &devices[2];
        assert_eq!(d3["name"], "iPhone 4S");
    }

    const HUAWEI_FIXTURE: &str = r#"<table class="wikitable"><tbody>
<tr><th>Model</th><th>Codename</th><th>Released</th></tr>
<tr><td>Huawei P30 Pro</td><td>VOG-L29</td><td>2019</td></tr>
<tr><td>Huawei Mate 60 Pro</td><td>ALN-AL10</td><td>2023</td></tr>
</tbody></table>"#;


    #[test]
    fn date_math() {
        assert_eq!(mid_date("2000-01-01", "2026-12-31"), "2013-07-01");
        assert_eq!(add_days("2026-12-31", 1), "2027-01-01");
        assert_eq!(mid_date("2026-01-01", "2026-12-31"), "2026-07-02");
        assert_eq!(add_days("2000-01-01", -1), "1999-12-31");
    }

    #[test]
    fn parse_huawei_fixture() {
        let tables = parse_wiki_tables(HUAWEI_FIXTURE);
        let devices = extract_from_tables(&tables, "Huawei", "huawei", None);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0]["name"], "Huawei P30 Pro");
        assert_eq!(devices[0]["codename"], "VOG-L29");
        assert_eq!(devices[0]["models"][0]["ids"][0], "VOG-L29");
        assert_eq!(devices[1]["name"], "Huawei Mate 60 Pro");
    }

    pub(crate) const APPLE_SUPPORT_FIXTURE: &str = r#"<div><h2 class="gb-header">iPhone 15 Pro Max</h2>
<p>Model numbers: A2849 (United States, Puerto Rico), A3105 (Canada), A3108 (China mainland)</p>
<h2 class="gb-header">iPhone 15</h2>
<p>Model numbers: A3089, A3092, A3090</p>
<h2 class="gb-header">iPhone SE (2nd generation)</h2>
<p>Model numbers: A2275 (United States)</p>
<p>Find the model number on the back: A1234 example text</p>"#;

    #[test]
    fn parse_apple_support_fixture() {
        let pairs = parse_apple_support(super::tests::APPLE_SUPPORT_FIXTURE);
        assert_eq!(pairs.len(), 3, "instruction text without a header must be skipped");
        let (name, ids) = &pairs[0];
        assert_eq!(name, "iPhone 15 Pro Max");
        assert_eq!(ids, &vec!["A2849".to_string(), "A3105".to_string(), "A3108".to_string()]);
        assert_eq!(pairs[1].0, "iPhone 15");
        assert_eq!(pairs[2].0, "iPhone SE (2nd generation)");
    }

    const APPLE_COLUMNS_FIXTURE: &str = r#"<table class="wikitable"><tbody>
<tr><th>Model</th><th>iPhone 5c</th><th>iPhone 5</th><th>iPhone 4s</th><th>iPhone 4</th></tr>
<tr><td>Picture</td><td></td><td></td><td></td><td></td></tr>
<tr><td>Basic Info</td><td>Hardware Strings</td><td>iPhone5,3iPhone5,4</td><td>iPhone5,1iPhone5,2</td><td>iPhone4,1</td><td>iPhone3,1</td></tr>
<tr><td>Model number</td><td>A1456A1507A1529</td><td>A1428A1429A1442</td><td>A1431A1387</td><td>A1349A1332</td></tr>
</tbody></table>"#;

    #[test]
    fn parse_apple_columns_fixture() {
        let tables = parse_wiki_tables(APPLE_COLUMNS_FIXTURE);
        assert_eq!(tables.len(), 1);
        let devices = extract_apple_columns(&tables);
        assert_eq!(devices.len(), 4);
        assert_eq!(devices[0]["name"], "iPhone 5c");
        assert_eq!(devices[0]["models"][0]["ids"], json!(["A1456", "A1507", "A1529"]));
        assert_eq!(devices[3]["name"], "iPhone 4");
        assert_eq!(devices[3]["models"][0]["ids"], json!(["A1332", "A1349"]), "A1332 老机型回归");
        assert_eq!(devices[3]["codename"], "iPhone3,1");
    }
}
