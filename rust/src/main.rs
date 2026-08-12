mod embed;
mod kv;
mod model;
mod parser;
mod server;
mod vector;

use anyhow::Result;
use model::Device;
use std::path::{Path, PathBuf};
use std::process::exit;

const DEFAULT_DATA_DIR: &str = "data";

fn usage() -> ! {
    eprintln!(
        r#"mobilemodels-db — MobileModels → redb KV + usearch vector DBs

USAGE:
  mobilemodels-db build [--data-dir DIR] [--source PATH]      # parse JSON -> KV + vector index
                                                               # --source: JSON 文件或含 *.json 的目录（默认当前目录）
  mobilemodels-db query <model|code|codename|name|brand|series> <KEY> [SERIES] [--data-dir DIR]
  mobilemodels-db search <TEXT> [-k N] [--brand NAME] [--data-dir DIR]
  mobilemodels-db export <file.json> [--data-dir DIR]
  mobilemodels-db serve [--host 127.0.0.1] [--port 8080] [--data-dir DIR]
  mobilemodels-db stats [--data-dir DIR]

Run from the repository root (or pass --data-dir). Default DIR = data/
"#
    );
    exit(1);
}

fn main() {
    // Piping into `head`/`cut` closes stdout early — that's a normal CLI
    // condition, not a bug. Swallow the stdio panic it triggers.
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if payload.contains("failed printing to stdout") {
            std::process::exit(0);
        }
        eprintln!("{info}");
    }));

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    let result = match args[1].as_str() {
        "build" => cmd_build(&args[2..]),
        "query" => cmd_query(&args[2..]),
        "search" => cmd_search(&args[2..]),
        "export" => cmd_export(&args[2..]),
        "serve" => cmd_serve(&args[2..]),
        "stats" => cmd_stats(&args[2..]),
        _ => usage(),
    };
    if let Err(e) = result {
        // A closed pipe (e.g. piping into `head`) is not a real error.
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::BrokenPipe {
                return;
            }
        }
        eprintln!("error: {e:#}");
        exit(1);
    }
}

/// Pull out a `--flag value` pair, returning the value and the remaining args.
fn take_flag(args: &[String], name: &str, default: &str) -> (String, Vec<String>) {
    let mut out = default.to_string();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name && i + 1 < args.len() {
            out = args[i + 1].clone();
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (out, rest)
}

fn data_paths(data_dir: &str) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(data_dir);
    (dir.join("mobilemodels.redb"), dir.join("vector.index"))
}

/// Render a device as one-line JSON.
fn device_json(d: &Device) -> String {
    serde_json::to_string(d).unwrap_or_default()
}
// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

fn cmd_build(args: &[String]) -> Result<()> {
    let (data_dir, rest) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    let (source_dir, _) = take_flag(&rest, "--source", ".");
    let (kv_path, _vec_path) = data_paths(&data_dir);

    let mut devices = collect_devices(Path::new(&source_dir))?;
    for (i, d) in devices.iter_mut().enumerate() {
        d.id = (i + 1) as u32;
    }
    println!("\nparsed {} devices total", devices.len());

    let kv = kv::KvStore::create(&kv_path)?;
    kv.build(&devices)?;
    println!("KV written  -> {}", kv_path.display());

    // Embed everything and persist the vectors in the KV file.
    let vectors: Vec<(u32, Vec<f32>)> = devices
        .iter()
        .map(|d| (d.id, embed::embed(&d.search_text())))
        .collect();
    kv.write_vectors(&vectors)?;
    println!(
        "vectors done -> {} ({} vectors x {} dims)",
        kv_path.display(),
        vectors.len(),
        embed::DIM
    );

    // Warm-build the HNSW index once to prove the graph builds cleanly.
    let idx = vector::VectorIndex::build(&vectors)?;
    println!("HNSW index    -> {} nodes (hnsw_rs, DistCosine)", idx.size());

    let s = kv.stats()?;
    println!(
        "stats: devices={} model_ids={} codenames={} built_at={}",
        s.devices, s.model_ids, s.codenames, s.built_at
    );
    Ok(())
}

/// Load devices from a JSON file or a directory of `*.json` files.
fn collect_devices(source: &Path) -> Result<Vec<Device>> {
    parser::load_devices(source)
}

// ---------------------------------------------------------------------------
// query / search / stats
// ---------------------------------------------------------------------------

fn cmd_query(args: &[String]) -> Result<()> {
    let (data_dir, rest) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    if rest.len() < 2 {
        usage();
    }
    let (kv_path, _) = data_paths(&data_dir);
    let kv = kv::KvStore::open(&kv_path)?;
    let kind = rest[0].as_str();
    let key = rest[1].as_str();
    let ids: Vec<u64> = match kind {
        "model" => kv.by_model_id(key)?,
        "code" => kv.by_code(key)?,
        "codename" => kv.by_codename(key)?,
        "name" => kv.by_name(key)?,
        "brand" => {
            let mut ids = kv.by_brand(key)?;
            if ids.is_empty() {
                let brands = kv.brands_containing(key)?;
                if !brands.is_empty() {
                    println!("(exact brand \"{key}\" not found; matching {})", brands.join(", "));
                    for b in brands {
                        ids.extend(kv.by_brand(&b)?);
                    }
                }
            }
            ids
        }
        "series" => {
            if rest.len() < 3 {
                usage();
            }
            let mut ids = kv.by_series(key, &rest[2])?;
            if ids.is_empty() {
                // exact miss -> fuzzy brand + substring series
                let brands = kv.brands_containing(key)?;
                let mut noted = false;
                for b in &brands {
                    let hits = kv.series_contains(Some(b), &rest[2])?;
                    if !hits.is_empty() {
                        if !noted {
                            println!("(matched brand(s): {})", brands.join(", "));
                            noted = true;
                        }
                        ids.extend(hits);
                    }
                }
            }
            ids
        }
        _ => usage(),
    };
    if ids.is_empty() {
        println!("no results for {kind} = \"{key}\"");
        return Ok(());
    }
    println!("{} result(s) for {kind} = \"{key}\":", ids.len());
    for id in ids {
        if let Some(d) = kv.get_device(id as u32)? {
            println!("{}", d.summary());
        }
    }
    Ok(())
}

fn cmd_search(args: &[String]) -> Result<()> {
    let (data_dir, rest) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    let (k_str, rest) = take_flag(&rest, "-k", "10");
    let (brand_filter, rest) = take_flag(&rest, "--brand", "");
    if rest.is_empty() {
        usage();
    }
    let text = rest.join(" ");
    let k: usize = k_str.parse().unwrap_or(10);

    let (kv_path, _) = data_paths(&data_dir);
    let kv = kv::KvStore::open(&kv_path)?;
    let vectors = kv.read_vectors()?;
    if vectors.is_empty() {
        anyhow::bail!("no vectors in {} — run `build` first", kv_path.display());
    }
    let idx = vector::VectorIndex::build(&vectors)?;

    let q = embed::embed(&text);
    // Resolve fuzzy brand filter (e.g. "小米" -> ["小米", "小米 (Xiaomi)"]).
    let brand_set: Vec<String> = if brand_filter.is_empty() {
        Vec::new()
    } else {
        let mut set = kv.brands_containing(&brand_filter)?;
        if set.is_empty() {
            set.push(brand_filter.clone());
        }
        set
    };
    // Search extra candidates so filtering doesn't starve the result.
    let probe = k * 8;
    let hits = idx.search(&q, probe);
    println!("top {k} semantic matches for \"{text}\"{}",
        if brand_set.is_empty() { String::new() } else { format!(" (brand: {})", brand_set.join(", ")) });
    let mut shown = 0;
    for (label, dist) in hits {
        let Some(d) = kv.get_device(label)? else { continue };
        if !brand_set.is_empty() && !brand_set.iter().any(|b| *b == d.brand) {
            continue;
        }
        let sim = 1.0 - dist;
        println!("  [sim {:.4}] {}", sim, d.summary().replace('\n', "\n             "));
        shown += 1;
        if shown >= k {
            break;
        }
    }
    if shown == 0 {
        println!("  (no matches)");
    }
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<()> {
    let (data_dir, rest) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    if rest.is_empty() {
        usage();
    }
    let out_path = &rest[0];
    let (kv_path, _) = data_paths(&data_dir);
    let kv = kv::KvStore::open(&kv_path)?;
    let devices = kv.all_devices()?;
    let mut out = String::with_capacity(devices.len() * 512);
    out.push('[');
    for (i, d) in devices.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&device_json(d));
    }
    out.push(']');
    std::fs::write(out_path, out.as_bytes())?;
    println!("exported {} devices -> {}", devices.len(), out_path);
    Ok(())
}

/// HTTP API server — axum on the tokio multi-thread runtime.
///
/// 12-factor friendly: when the `PORT` env var is set (Render / Cloud Run /
/// Fly.io / Railway convention) it binds 0.0.0.0:PORT by default; explicit
/// `--host`/`--port` flags always win.
fn cmd_serve(args: &[String]) -> Result<()> {
    let (data_dir, rest) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    let env_port = std::env::var("PORT").ok();
    let default_host = if env_port.is_some() { "0.0.0.0" } else { "127.0.0.1" };
    let (host, rest) = take_flag(&rest, "--host", default_host);
    let default_port = env_port.as_deref().unwrap_or("8080");
    let (port, _) = take_flag(&rest, "--port", default_port);
    let port: u16 = port.parse().map_err(|_| anyhow::anyhow!("invalid port: {port}"))?;

    let (kv_path, _) = data_paths(&data_dir);
    let kv = kv::KvStore::open(&kv_path)?;
    let vectors = kv.read_vectors()?;
    if vectors.is_empty() {
        eprintln!("warning: no vectors in {} — empty dataset, search will return nothing", kv_path.display());
    }
    let started = std::time::Instant::now();
    let index = vector::VectorIndex::build(&vectors)?;
    println!("HNSW index ready: {} nodes in {:?}", index.size(), started.elapsed());

    let state = std::sync::Arc::new(server::AppState { kv, index });
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(server::serve(state, &host, port))
}

fn cmd_stats(args: &[String]) -> Result<()> {
    let (data_dir, _) = take_flag(args, "--data-dir", DEFAULT_DATA_DIR);
    let (kv_path, _) = data_paths(&data_dir);
    let kv = kv::KvStore::open(&kv_path)?;
    let s = kv.stats()?;
    println!("built_at:   {}", s.built_at);
    println!("devices:    {}", s.devices);
    println!("model_ids:  {}", s.model_ids);
    println!("codenames:  {}", s.codenames);
    println!("per brand:");
    for (b, n) in &s.per_brand {
        println!("  {:8} {}", n, b);
    }
    Ok(())
}
