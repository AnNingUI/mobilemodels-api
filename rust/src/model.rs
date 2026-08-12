use serde::{Deserialize, Serialize};

/// One model row, e.g. `` `A1332`: iPhone 4 `` or `` `E2104` `E2105`: Xperia E4 ``
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub ids: Vec<String>,
    pub market_name: String,
}

/// A device entry parsed from the markdown: `**[`code`] name (`codename`):**`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Sequential id, also used as the usearch vector label.
    pub id: u32,
    pub brand: String,
    /// Source file name, e.g. "apple_all.json" / "brands/example.json"
    pub file: String,
    /// `##` section heading, empty when the file has no sections.
    pub series: String,
    /// `[`code`]` device code, e.g. N82AP / X1. Empty when absent.
    pub code: String,
    /// Display name, e.g. "iPhone 4" / "小米 1"
    pub name: String,
    /// `(`codename`)`, e.g. "iPhone3,1" / "mione_plus". Empty when absent.
    pub codename: String,
    pub models: Vec<Model>,
}

impl Device {
    /// Full text used for embedding / semantic search.
    pub fn search_text(&self) -> String {
        let mut parts = vec![
            self.brand.clone(),
            self.series.clone(),
            self.name.clone(),
            self.code.clone(),
            self.codename.clone(),
        ];
        for m in &self.models {
            for id in &m.ids {
                parts.push(id.clone());
            }
            parts.push(m.market_name.clone());
        }
        parts.join(" ")
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "#{} [{}] {} (series: {}, code: {}, codename: {})",
            self.id, self.brand, self.name, self.series, self.code, self.codename
        );
        if !self.models.is_empty() {
            let sample: Vec<String> = self
                .models
                .iter()
                .take(4)
                .map(|m| format!("{}: {}", m.ids.join("/"), m.market_name))
                .collect();
            s.push_str(&format!("\n    models ({}): {}", self.models.len(), sample.join(" | ")));
        }
        s
    }
}
