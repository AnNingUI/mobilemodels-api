use crate::model::{Device, Model};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Remove zero-width / soft-hyphen characters that sneak into some lines
/// (e.g. `**大神 Note 3:​**` contains U+200B before the closing `**`).
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}'))
        .collect()
}

/// Parse a device header line into (code, name, codename).
///
/// Accepted forms (after cleaning):
///   **[`N82AP`] iPhone 3G (`iPhone1,2`):**
///   **Pixel (`sailfish`):**
///   **中兴天机 7:**
///   **[`A1Pro`] 小米 5 高配版 (`gold`)**      <- missing trailing colon
fn parse_device_header(line: &str) -> Option<(String, String, String)> {
    let s = clean(line.trim());
    if !s.starts_with("**") {
        return None;
    }
    if !(s.ends_with(":**") || s.ends_with(")**")) {
        return None;
    }
    let inner = &s[2..s.len() - 3]; // strip "**" prefix and ":**" / ")**" suffix
    let mut rest = inner;

    let mut code = String::new();
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let end = after_bracket.find(']')?;
        code = after_bracket[..end].trim().trim_matches('`').to_string();
        rest = after_bracket[end + 1..].trim_start();
    }

    let mut name = rest.trim();
    let mut codename = String::new();
    if name.ends_with(')') {
        if let Some(open) = name.rfind('(') {
            // A real codename suffix: last '(' ... ')' at the very end
            let cand = name[open + 1..name.len() - 1].trim();
            codename = cand.trim_matches('`').to_string();
            name = name[..open].trim_end();
        }
    }
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((code, name.to_string(), codename))
}

/// Parse one model row: `` `ID1` `ID2`: Market Name ``
/// Also accepts bare `` `ID` `` lines (used by the misc early-model files).
fn parse_model_line(line: &str) -> Option<Model> {
    let s = clean(line.trim());
    if !s.starts_with('`') {
        return None;
    }
    let parts: Vec<&str> = s.split('`').collect();
    let mut ids = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i % 2 == 1 {
            let t = p.trim();
            if !t.is_empty() {
                ids.push(t.to_string());
            }
        }
    }
    if ids.is_empty() {
        return None;
    }
    let tail = parts.last().copied().unwrap_or("");
    let market_name = tail
        .trim_start_matches(':')
        .trim()
        .trim_matches('~')
        .trim();
    let market_name = if market_name.is_empty() {
        ids[0].clone()
    } else {
        market_name.to_string()
    };
    Some(Model { ids, market_name })
}

/// Standard brand-file mode: `##` sections + `**[...] name (codename):**` headers
/// + `` `id`: name `` rows.
pub fn parse_brand_file(path: &Path, brand: &str, file: &str) -> Result<Vec<Device>> {
    let content = fs::read_to_string(path)?;
    let mut devices = Vec::new();
    let mut series = String::new();
    let mut current: Option<Device> = None;

    for raw in content.lines() {
        let t = clean(raw.trim());
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("## ") {
            series = rest.trim().to_string();
            continue;
        }
        if t.starts_with('#') || t.starts_with("- ") || t.starts_with('|') || t.starts_with('>') || t.starts_with('~') {
            continue;
        }
        if t.starts_with("**") {
            if let Some(dev) = current.take() {
                devices.push(dev);
            }
            if let Some((code, name, codename)) = parse_device_header(&t) {
                current = Some(Device {
                    id: 0,
                    brand: brand.to_string(),
                    file: file.to_string(),
                    series: series.clone(),
                    code,
                    name,
                    codename,
                    models: Vec::new(),
                });
            }
            continue;
        }
        if let Some(dev) = current.as_mut() {
            if let Some(model) = parse_model_line(&t) {
                dev.models.push(model);
            }
        }
    }
    if let Some(dev) = current.take() {
        devices.push(dev);
    }
    Ok(devices)
}

/// List mode for `misc/early-*.md`: bare `` `ID` `` / `` `ID`: Name `` lines,
/// each line becomes its own device so model-id lookup stays uniform.
pub fn parse_list_file(path: &Path, brand: &str, file: &str, series_label: &str) -> Result<Vec<Device>> {
    let content = fs::read_to_string(path)?;
    let mut devices = Vec::new();
    for raw in content.lines() {
        let t = clean(raw.trim());
        if t.is_empty() || t.starts_with('#') || t.starts_with("- ") {
            continue;
        }
        if let Some(model) = parse_model_line(&t) {
            let name = model.market_name.clone();
            devices.push(Device {
                id: 0,
                brand: brand.to_string(),
                file: file.to_string(),
                series: series_label.to_string(),
                code: String::new(),
                name,
                codename: String::new(),
                models: vec![model],
            });
        }
    }
    Ok(devices)
}

/// Table mode for `misc/xiaomi-book-internal-names.md`:
/// `| 名称 | 编号/代号 | 上市年份 |` — first data column is the name,
/// second is the internal code/codename.
pub fn parse_table_file(path: &Path, brand: &str, file: &str, series_label: &str) -> Result<Vec<Device>> {
    let content = fs::read_to_string(path)?;
    let mut devices = Vec::new();
    let mut seen_header = false;
    for raw in content.lines() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('|') {
            let cols: Vec<&str> = t
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim())
                .collect();
            if cols.iter().any(|c| c.contains('-')) {
                seen_header = true; // separator row `| :-: | :-: |`
                continue;
            }
            if !seen_header || cols.len() < 2 || cols[0].is_empty() {
                continue;
            }
            let code = cols[1].trim_matches('`');
            devices.push(Device {
                id: 0,
                brand: brand.to_string(),
                file: file.to_string(),
                series: series_label.to_string(),
                code: if code.is_empty() || code == "--" { String::new() } else { code.to_string() },
                name: cols[0].to_string(),
                codename: String::new(),
                models: Vec::new(),
            });
        }
    }
    Ok(devices)
}
