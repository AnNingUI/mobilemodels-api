//! HTTP API server — axum (tokio) 高性能高并发接口层。
//!
//! The HNSW index and the redb store live in an `Arc<AppState>` and are
//! shared, read-only across all worker tasks: every request is served
//! concurrently with zero locking on the hot path (redb MVCC read
//! transactions + hnsw_rs read-locked ANN search).

use crate::embed;
use crate::kv::KvStore;
use crate::vector::ExactIndex;
use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

pub struct AppState {
    pub kv: KvStore,
    pub index: ExactIndex,
}

pub type Shared = Arc<AppState>;

pub fn app(state: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/devices/{id}", get(device_by_id))
        .route("/query/{kind}/{key}", get(query))
        .route("/query/series/{brand}/{series}", get(query_series))
        .route("/search", get(search))
        .route("/export", get(export_all))
        .with_state(state)
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

fn device_json(d: &crate::model::Device) -> Value {
    serde_json::to_value(d).unwrap_or(Value::Null)
}

/// Resolve ids -> device JSON values, preserving order and skipping gaps.
fn devices_from_ids(kv: &KvStore, ids: &[u64]) -> Vec<Value> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(Some(d)) = kv.get_device(*id as u32) {
            out.push(device_json(&d));
        }
    }
    out
}

async fn health(State(s): State<Shared>) -> Response {
    let count = s
        .kv
        .stats()
        .map(|st| st.devices)
        .unwrap_or_default();
    Json(json!({
        "status": "ok",
        "service": "mobilemodels-db",
        "devices": count,
        "vector_dim": embed::DIM,
        "index_nodes": s.index.size(),
    }))
    .into_response()
}

async fn stats(State(s): State<Shared>) -> Response {
    match s.kv.stats() {
        Ok(st) => Json(json!({
            "built_at": st.built_at,
            "devices": st.devices,
            "model_ids": st.model_ids,
            "codenames": st.codenames,
            "index_nodes": s.index.size(),
            "per_brand": st.per_brand.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    }
}

async fn device_by_id(State(s): State<Shared>, Path(id): Path<u32>) -> Response {
    match s.kv.get_device(id) {
        Ok(Some(d)) => Json(device_json(&d)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, &format!("device {id} not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    }
}

async fn query(State(s): State<Shared>, Path((kind, key)): Path<(String, String)>) -> Response {
    let kv = &s.kv;
    let ids: Vec<u64> = match kind.as_str() {
        "model" => kv.by_model_id(&key).unwrap_or_default(),
        "code" => kv.by_code(&key).unwrap_or_default(),
        "codename" => kv.by_codename(&key).unwrap_or_default(),
        "name" => kv.by_name(&key).unwrap_or_default(),
        "brand" => {
            let mut ids = kv.by_brand(&key).unwrap_or_default();
            if ids.is_empty() {
                if let Ok(brands) = kv.brands_containing(&key) {
                    for b in brands {
                        ids.extend(kv.by_brand(&b).unwrap_or_default());
                    }
                }
            }
            ids
        }
        other => return err(StatusCode::BAD_REQUEST, &format!("unknown kind `{other}` (model|code|codename|name|brand)")),
    };
    let matches = devices_from_ids(kv, &ids);
    Json(json!({
        "kind": kind,
        "key": key,
        "count": matches.len(),
        "matches": matches,
    }))
    .into_response()
}

async fn query_series(
    State(s): State<Shared>,
    Path((brand, series)): Path<(String, String)>,
) -> Response {
    let kv = &s.kv;
    let mut ids = kv.by_series(&brand, &series).unwrap_or_default();
    let mut fuzzy = false;
    if ids.is_empty() {
        fuzzy = true;
        if let Ok(brands) = kv.brands_containing(&brand) {
            for b in brands {
                if let Ok(hits) = kv.series_contains(Some(&b), &series) {
                    ids.extend(hits);
                }
            }
        }
    }
    let matches = devices_from_ids(kv, &ids);
    Json(json!({
        "kind": "series",
        "brand": brand,
        "series": series,
        "fuzzy": fuzzy,
        "count": matches.len(),
        "matches": matches,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default = "default_k")]
    k: usize,
    #[serde(default)]
    brand: Option<String>,
}

fn default_k() -> usize {
    10
}

async fn search(
    State(s): State<Shared>,
    Query(p): Query<SearchParams>,
) -> Response {
    let started = Instant::now();
    let k = p.k.clamp(1, 100);
    let kv = &s.kv;

    // Resolve fuzzy brand filter once per request.
    let brand_set: Vec<String> = match &p.brand {
        None => Vec::new(),
        Some(b) => {
            let mut set = kv.brands_containing(b).unwrap_or_default();
            if set.is_empty() {
                set.push(b.clone());
            }
            set
        }
    };

    let q = embed::embed(&p.q);

    let mut results: Vec<Value> = Vec::new();
    let mut seen_ids: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let pass_brand = |d: &crate::model::Device| {
        brand_set.is_empty() || brand_set.iter().any(|b| *b == d.brand)
    };

    // 1) 精确提升：查询文本完全匹配型号/代号/名称时置顶（避免哈希碰撞误排）
    let q_trim = p.q.trim().to_string();
    if !q_trim.is_empty() {
        for ids in [
            kv.by_model_id(&q_trim),
            kv.by_codename(&q_trim),
            kv.by_name(&q_trim),
        ] {
            for id in ids.unwrap_or_default() {
                if results.len() >= k {
                    break;
                }
                if !seen_ids.insert(id) {
                    continue;
                }
                if let Ok(Some(d)) = kv.get_device(id as u32) {
                    if pass_brand(&d) {
                        results.push(json!({
                            "id": d.id,
                            "similarity": 1.0,
                            "device": device_json(&d),
                        }));
                    }
                }
            }
        }
    }

    // 2) 向量语义结果（补充，跳过已列出的）
    let probe = (k * 8).max(32);
    let hits = s.index.search(&q, probe);
    for (label, dist) in hits {
        if results.len() >= k {
            break;
        }
        if !seen_ids.insert(label as u64) {
            continue;
        }
        let Ok(Some(d)) = kv.get_device(label) else { continue };
        if !pass_brand(&d) {
            continue;
        }
        results.push(json!({
            "id": d.id,
            "similarity": (1.0 - dist).max(0.0),
            "device": device_json(&d),
        }));
    }

    Json(json!({
        "query": p.q,
        "k": k,
        "brand_filter": brand_set,
        "count": results.len(),
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "results": results,
    }))
    .into_response()
}

async fn export_all(State(s): State<Shared>) -> Response {
    match s.kv.all_devices() {
        Ok(devices) => {
            let arr: Vec<Value> = devices.iter().map(device_json).collect();
            Json(arr).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    }
}

/// Run the server (called from a tokio runtime).
pub async fn serve(state: Shared, host: &str, port: u16) -> Result<()> {
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("mobilemodels-db API listening on http://{addr}");
    println!("  GET /health  GET /stats  GET /devices/{{id}}");
    println!("  GET /query/{{model|code|codename|name|brand}}/{{key}}");
    println!("  GET /query/series/{{brand}}/{{series}}  GET /search?q=..&k=..&brand=..  GET /export");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
