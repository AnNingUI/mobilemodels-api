//! HNSW vector index (pure Rust, via `hnsw_rs`) — the vector layer.
//!
//! Vectors are persisted in the redb KV file; the HNSW graph is rebuilt
//! on demand (fast for this dataset size) so there is exactly one durable
//! artifact: `data/mobilemodels.redb`.

use anyhow::Result;
use anndists::dist::DistCosine;
use hnsw_rs::prelude::*;

pub struct VectorIndex {
    index: Hnsw<'static, f32, DistCosine>,
}

impl VectorIndex {
    /// Build an HNSW graph from (device_id, embedding) pairs.
    /// Embeddings must be L2-normalized (embed::embed guarantees this);
    /// DistDot then equals cosine distance = 1 - similarity.
    pub fn build(vectors: &[(u32, Vec<f32>)]) -> Result<Self> {
        let n = vectors.len().max(1);
        let max_layer = ((n as f32).ln().trunc() as usize).clamp(1, 16);
        let index = Hnsw::<f32, DistCosine>::new(16, n, max_layer, 100, DistCosine::default());
        let with_id: Vec<(&[f32], usize)> = vectors
            .iter()
            .map(|(id, v)| (v.as_slice(), *id as usize))
            .collect();
        index.parallel_insert_slice(&with_id);
        Ok(Self { index })
    }

    /// k nearest neighbours, returned as (device_id, distance).
    /// Distance = 1 - cosine similarity for normalized vectors.

    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u32, f32)> {
        let ef = (k * 4).max(32);
        self.index
            .search(query, k, ef)
            .into_iter()
            .map(|n| (n.get_origin_id() as u32, n.distance))
            .collect()
    }

    pub fn size(&self) -> usize {
        self.index.get_nb_point()
    }
}

/// 精确线性扫描索引 —— 供服务端使用。
/// 51k x 1024 维点积 ~几毫秒，**无需构建 HNSW 图**：启动即就绪、精确召回，
/// 适合免费平台（0.1 CPU / 512MB）秒级冷启动。
pub struct ExactIndex {
    vectors: Vec<(u32, Vec<f32>)>,
}

impl ExactIndex {
    pub fn new(vectors: Vec<(u32, Vec<f32>)>) -> Self {
        Self { vectors }
    }


    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u32, f32)> {
        let mut scored: Vec<(u32, f32)> = self
            .vectors
            .iter()
            .map(|(id, v)| {
                let dot: f32 = v.iter().zip(query.iter()).map(|(a, b)| a * b).sum();
                (*id, 1.0 - dot) // 距离 = 1 - cos
            })
            .collect();
        // 完整排序（51k 条仅几毫秒）；select_nth 不保证顺序，服务端会取错
        scored.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        scored.truncate(k);
        scored
    }

    pub fn size(&self) -> usize {
        self.vectors.len()
    }

    /// 暴露内部向量（调试用）
    pub fn vectors(&self) -> &Vec<(u32, Vec<f32>)> {
        &self.vectors
    }
}
