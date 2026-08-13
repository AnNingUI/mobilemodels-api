//! Deterministic n-gram feature-hashing embeddings.
//!
//! Pure Rust, zero model downloads, instant to compute. Character unigrams +
//! bigrams (critical for CJK) plus whole ASCII tokens (critical for model ids
//! like `A1332` and codenames like `mione_plus`), hashed with FNV-1a into a
//! fixed-dimension vector and L2-normalized for cosine similarity.

pub const DIM: usize = 512;

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c.to_ascii_lowercase() } else { c })
        .collect()
}

pub fn embed(text: &str) -> Vec<f32> {
    let s = normalize(text);
    let mut vec = vec![0f32; DIM];

    let mut add_feature = |feature: &str, weight: f32| {
        let h = fnv1a(feature.as_bytes(), 0);
        let idx = ((h >> 1) % DIM as u64) as usize;
        let sign = if h & 1 == 1 { 1.0 } else { -1.0 };
        vec[idx] += sign * weight;
    };

    // Whole ASCII alphanumeric tokens (model ids, codenames): strongest signal.
    let mut tok = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            tok.push(c);
        } else if !tok.is_empty() {
            add_feature(&tok, 4.0);
            tok.clear();
        }
    }
    if !tok.is_empty() {
        add_feature(&tok, 4.0);
    }

    // Char unigrams + bigrams (CJK-friendly), skipping whitespace crossings.
    let chars: Vec<char> = s.chars().collect();
    for w in chars.windows(2) {
        if w[0].is_whitespace() || w[1].is_whitespace() {
            continue;
        }
        let mut bigram = String::with_capacity(8);
        bigram.push(w[0]);
        bigram.push(w[1]);
        add_feature(&bigram, 2.0);
    }
    for &c in &chars {
        if !c.is_whitespace() {
            let mut uni = String::with_capacity(4);
            uni.push(c);
            add_feature(&uni, 0.5);
        }
    }

    // L2 normalize -> cosine similarity = dot product in usearch.
    let norm = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
    vec
}
