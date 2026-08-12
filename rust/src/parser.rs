//! JSON 数据解析器 —— 唯一输入格式。
//!
//! 一个 JSON 文件即一个设备数组：
//!
//! ```json
//! [
//!   {
//!     "brand": "Apple",
//!     "series": "iPhone",
//!     "code": "N90AP",
//!     "name": "iPhone 4",
//!     "codename": "iPhone3,1",
//!     "models": [
//!       { "ids": ["A1332"], "market_name": "iPhone 4 (GSM)" },
//!       "A1333"
//!     ]
//!   }
//! ]
//! ```
//!
//! - `brand`/`series`/`code`/`codename`/`file` 均可省略（`brand` 缺省用文件名，其余用空串）
//! - `models` 可省略；每项支持三种写法：`"ID"`、`["ID1","ID2"]`、`{"ids":[...],"market_name":".."}`，
//!   `market_name` 缺省取设备名 `name`

use crate::model::{Device, Model};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ModelInput {
    One(String),
    Ids(Vec<String>),
    Full {
        ids: Vec<String>,
        market_name: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct DeviceInput {
    brand: Option<String>,
    /// 来源文件标记（可覆盖；缺省为实际文件名）
    file: Option<String>,
    series: Option<String>,
    code: Option<String>,
    name: String,
    codename: Option<String>,
    #[serde(default)]
    models: Vec<ModelInput>,
}

/// Parse one JSON file (array of devices) into `Device`s.
/// `default_brand` is used for devices without an explicit `brand` field.
pub fn parse_json_file(path: &Path, default_brand: &str) -> Result<Vec<Device>> {
    let content = std::fs::read_to_string(path)?;
    let inputs: Vec<DeviceInput> = serde_json::from_str(&content)
        .with_context(|| format!("parsing {}", path.display()))?;
    let file = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut out = Vec::with_capacity(inputs.len());
    for d in inputs {
        let fallback_name = d.name.clone();
        let models = d
            .models
            .into_iter()
            .filter_map(|m| match m {
                ModelInput::One(id) => {
                    let id = id.trim().to_string();
                    (!id.is_empty()).then(|| Model {
                        ids: vec![id],
                        market_name: fallback_name.clone(),
                    })
                }
                ModelInput::Ids(ids) => {
                    let ids: Vec<String> = ids.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    (!ids.is_empty()).then(|| Model {
                        ids,
                        market_name: fallback_name.clone(),
                    })
                }
                ModelInput::Full { ids, market_name } => {
                    let ids: Vec<String> = ids.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    (!ids.is_empty()).then(|| Model {
                        ids,
                        market_name: market_name.unwrap_or_else(|| fallback_name.clone()),
                    })
                }
            })
            .collect();

        out.push(Device {
            id: 0,
            brand: d.brand.unwrap_or_else(|| default_brand.to_string()),
            file: d.file.unwrap_or_else(|| file.clone()),
            series: d.series.unwrap_or_default(),
            code: d.code.unwrap_or_default(),
            name: d.name,
            codename: d.codename.unwrap_or_default(),
            models,
        });
    }
    Ok(out)
}

/// Load devices from a JSON file or a directory of `*.json` files.
pub fn load_devices(source: &Path) -> Result<Vec<Device>> {
    let mut devices = Vec::new();
    if source.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(source)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .collect();
        files.sort();
        for path in files {
            let default_brand = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let ds = parse_json_file(&path, &default_brand)?;
            println!("  {:40} {:5} devices", path.display(), ds.len());
            devices.extend(ds);
        }
    } else {
        let default_brand = source
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        devices = parse_json_file(source, &default_brand)?;
    }
    Ok(devices)
}
