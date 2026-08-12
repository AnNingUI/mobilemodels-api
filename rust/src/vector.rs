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
