use crate::model::Device;
use anyhow::Result;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;

const DEVICES: TableDefinition<u32, &[u8]> = TableDefinition::new("devices");
const BY_MODEL_ID: TableDefinition<&str, &[u8]> = TableDefinition::new("by_model_id");
const BY_NAME: TableDefinition<&str, &[u8]> = TableDefinition::new("by_name");
const BY_CODE: TableDefinition<&str, &[u8]> = TableDefinition::new("by_code");
const BY_CODENAME: TableDefinition<&str, &[u8]> = TableDefinition::new("by_codename");
const BY_BRAND: TableDefinition<&str, &[u8]> = TableDefinition::new("by_brand");
const BY_SERIES: TableDefinition<&str, &[u8]> = TableDefinition::new("by_series");
const VECTORS: TableDefinition<u32, &[u8]> = TableDefinition::new("vectors");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

/// ASCII-lowercase key normalization (model ids / codenames / names are
/// case-insensitive in practice: A1332 == a1332, X1 == x1).
pub fn norm(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c.to_ascii_lowercase() } else { c })
        .collect()
}

pub struct KvStore {
    db: Database,
}

fn append_id(table: &mut redb::Table<'_, &str, &[u8]>, key: &str, id: u32) -> Result<()> {
    let mut list: Vec<u64> = match table.get(key)? {
        Some(g) => bincode::deserialize(g.value())?,
        None => Vec::new(),
    };
    list.push(id as u64);
    table.insert(key, bincode::serialize(&list)?.as_slice())?;
    Ok(())
}

impl KvStore {
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self { db: Database::create(path)? })
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self { db: Database::open(path)? })
    }

    pub fn build(&self, devices: &[Device]) -> Result<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut dev = wtx.open_table(DEVICES)?;
            let mut by_mid = wtx.open_table(BY_MODEL_ID)?;
            let mut by_name = wtx.open_table(BY_NAME)?;
            let mut by_code = wtx.open_table(BY_CODE)?;
            let mut by_cn = wtx.open_table(BY_CODENAME)?;
            let mut by_brand = wtx.open_table(BY_BRAND)?;
            let mut by_series = wtx.open_table(BY_SERIES)?;
            let mut meta = wtx.open_table(META)?;

            for d in devices {
                let bytes = bincode::serialize(d)?;
                dev.insert(d.id, bytes.as_slice())?;
                append_id(&mut by_name, &norm(&d.name), d.id)?;
                if !d.code.is_empty() {
                    append_id(&mut by_code, &norm(&d.code), d.id)?;
                }
                if !d.codename.is_empty() {
                    append_id(&mut by_cn, &norm(&d.codename), d.id)?;
                }
                append_id(&mut by_brand, &d.brand, d.id)?;
                if !d.series.is_empty() {
                    append_id(&mut by_series, &format!("{}\u{0}{}", d.brand, d.series), d.id)?;
                }
                for m in &d.models {
                    for id in &m.ids {
                        append_id(&mut by_mid, &norm(id), d.id)?;
                    }
                }
            }
            meta.insert("device_count", devices.len().to_string().as_str())?;
            meta.insert("built_at", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default()
                .as_str())?;
        }
        wtx.commit()?;
        Ok(())
    }

    /// Persist embeddings (device_id -> vector) in the same redb file.
    pub fn write_vectors(&self, vectors: &[(u32, Vec<f32>)]) -> Result<()> {
        let wtx = self.db.begin_write()?;
        {
            let mut t = wtx.open_table(VECTORS)?;
            for (id, v) in vectors {
                t.insert(*id, bincode::serialize(v)?.as_slice())?;
            }
        }
        wtx.commit()?;
        Ok(())
    }

    /// Read all persisted embeddings back (for rebuilding the HNSW graph).
    pub fn read_vectors(&self) -> Result<Vec<(u32, Vec<f32>)>> {
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(VECTORS)?;
        let mut out = Vec::with_capacity(t.len()? as usize);
        for item in t.iter()? {
            let (k, v) = item?;
            out.push((k.value(), bincode::deserialize::<Vec<f32>>(v.value())?));
        }
        Ok(out)
    }

    pub fn get_device(&self, id: u32) -> Result<Option<Device>> {
        let rtx = self.db.begin_read()?;
        let table = rtx.open_table(DEVICES)?;
        match table.get(id)? {
            Some(g) => Ok(Some(bincode::deserialize(g.value())?)),
            None => Ok(None),
        }
    }

    /// All devices, ordered by id.
    pub fn all_devices(&self) -> Result<Vec<Device>> {
        let rtx = self.db.begin_read()?;
        let table = rtx.open_table(DEVICES)?;
        let mut out = Vec::with_capacity(table.len()? as usize);
        for item in table.iter()? {
            let (_, v) = item?;
            out.push(bincode::deserialize(v.value())?);
        }
        Ok(out)
    }

    fn ids_for(&self, table_name: &str, key: &str) -> Result<Vec<u64>> {
        let rtx = self.db.begin_read()?;
        let table = rtx.open_table(TableDefinition::<&str, &[u8]>::new(table_name))?;
        match table.get(key)? {
            Some(g) => Ok(bincode::deserialize(g.value())?),
            None => Ok(Vec::new()),
        }
    }

    pub fn by_model_id(&self, id: &str) -> Result<Vec<u64>> {
        self.ids_for("by_model_id", &norm(id))
    }
    pub fn by_name(&self, name: &str) -> Result<Vec<u64>> {
        self.ids_for("by_name", &norm(name))
    }
    pub fn by_code(&self, code: &str) -> Result<Vec<u64>> {
        self.ids_for("by_code", &norm(code))
    }
    pub fn by_codename(&self, cn: &str) -> Result<Vec<u64>> {
        self.ids_for("by_codename", &norm(cn))
    }
    pub fn by_brand(&self, brand: &str) -> Result<Vec<u64>> {
        self.ids_for("by_brand", brand)
    }

    /// Case-insensitive substring match over stored brand names.
    pub fn brands_containing(&self, sub: &str) -> Result<Vec<String>> {
        let sub = sub.to_lowercase();
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(BY_BRAND)?;
        let mut out = Vec::new();
        for item in t.iter()? {
            let (k, _) = item?;
            let brand = k.value();
            if brand.to_lowercase().contains(&sub) {
                out.push(brand.to_string());
            }
        }
        Ok(out)
    }
    pub fn by_series(&self, brand: &str, series: &str) -> Result<Vec<u64>> {
        self.ids_for("by_series", &format!("{}\u{0}{}", brand, series))
    }

    /// Case-insensitive substring match over stored series names.
    /// brand: optional filter (exact brand name, e.g. "华为").
    pub fn series_contains(&self, brand: Option<&str>, series_sub: &str) -> Result<Vec<u64>> {
        let sub = series_sub.to_lowercase();
        let rtx = self.db.begin_read()?;
        let t = rtx.open_table(BY_SERIES)?;
        let mut out = Vec::new();
        for item in t.iter()? {
            let (k, v) = item?;
            let key = k.value();
            if let Some(sep) = key.find('\u{0}') {
                let (b, s) = (&key[..sep], &key[sep + 1..]);
                if s.to_lowercase().contains(&sub)
                    && brand.map(|bq| b == bq).unwrap_or(true)
                {
                    out.extend(bincode::deserialize::<Vec<u64>>(v.value())?);
                }
            }
        }
        Ok(out)
    }

    pub fn stats(&self) -> Result<Stats> {
        let rtx = self.db.begin_read()?;
        let dev = rtx.open_table(DEVICES)?;
        let mid = rtx.open_table(BY_MODEL_ID)?;
        let cn = rtx.open_table(BY_CODENAME)?;
        let brand = rtx.open_table(BY_BRAND)?;
        let meta = rtx.open_table(META)?;
        let get_meta = |k: &str| -> String {
            meta.get(k).ok().flatten().map(|g| g.value().to_string()).unwrap_or_default()
        };
        let mut per_brand: Vec<(String, u64)> = Vec::new();
        for item in brand.iter()? {
            let (k, g) = item?;
            let n: Vec<u64> = bincode::deserialize(g.value())?;
            per_brand.push((k.value().to_string(), n.len() as u64));
        }
        per_brand.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(Stats {
            devices: dev.len()?,
            model_ids: mid.len()?,
            codenames: cn.len()?,
            built_at: get_meta("built_at"),
            per_brand,
        })
    }
}

pub struct Stats {
    pub devices: u64,
    pub model_ids: u64,
    pub codenames: u64,
    pub built_at: String,
    pub per_brand: Vec<(String, u64)>,
}
