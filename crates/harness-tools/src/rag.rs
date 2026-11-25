use chrono::{DateTime, Utc};
use harness_net::NetworkGate;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeChunk {
    pub id: String,
    pub rel_path: String,
    pub location: String,
    pub content: String,
    pub terms: HashMap<String, usize>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub workspace_path: String,
    pub total_files: usize,
    pub total_chunks: usize,
    pub has_dense_embeddings: bool,
    pub embedding_model: Option<String>,
    pub indexed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceKnowledgeIndex {
    pub stats: IndexStats,
    pub avg_doc_len: f64,
    pub doc_freqs: HashMap<String, usize>,
    pub chunks: Vec<KnowledgeChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchHit {
    pub rel_path: String,
    pub location: String,
    pub score: f64,
    pub snippet: String,
}

const SUPPORTED_EXTS: &[&str] = &[
    "md", "txt", "py", "rs", "ts", "tsx", "js", "jsx", "html", "htm", "tex", "json", "toml",
    "sql", "csv", "pdf",
];

fn index_file_path(workspace_root: &Path) -> std::path::PathBuf {
    workspace_root.join(".bumbo").join("knowledge_index.json")
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

fn term_counts(text: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for tok in tokenize(text) {
        *map.entry(tok).or_insert(0) += 1;
    }
    map
}

fn chunk_text(text: &str, chunk_chars: usize, overlap_chars: usize) -> Vec<(usize, String)> {
    let indices: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let total = indices.len();
    if total == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let step = chunk_chars.saturating_sub(overlap_chars).max(100);
    let mut start_ci = 0usize;
    let mut idx = 1usize;

    while start_ci < total {
        let end_ci = (start_ci + chunk_chars).min(total);
        let byte_start = indices[start_ci];
        let byte_end = if end_ci == total {
            text.len()
        } else {
            indices[end_ci]
        };
        let slice = text[byte_start..byte_end].trim().to_string();
        if !slice.is_empty() {
            chunks.push((idx, slice));
            idx += 1;
        }
        if end_ci == total {
            break;
        }
        start_ci += step;
    }
    chunks
}

pub async fn build_workspace_index(
    workspace_root: &Path,
    gate: &NetworkGate,
) -> anyhow::Result<IndexStats> {
    let mut chunks = Vec::new();
    let mut file_count = 0usize;

    let walker = WalkDir::new(workspace_root)
        .max_depth(5)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(
                n.as_ref(),
                ".git" | "target" | "node_modules" | "__pycache__" | ".venv" | ".bumbo" | ".cinder"
            )
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !SUPPORTED_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if entry.metadata().map(|m| m.len()).unwrap_or(0) > 6 * 1024 * 1024 {
            continue;
        }

        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if crate::risk::is_sensitive_secret_path(&rel_path) {
            continue;
        }

        if ext == "pdf" {
            if let Ok(doc) = lopdf::Document::load(path) {
                let mut page_nums: Vec<u32> = doc.get_pages().keys().copied().collect();
                page_nums.sort_unstable();
                page_nums.truncate(40);
                let mut added_any = false;
                for p in page_nums {
                    if let Ok(txt) = doc.extract_text(&[p]) {
                        for (part_i, slice) in chunk_text(&txt, 700, 100) {
                            let terms = term_counts(&slice);
                            chunks.push(KnowledgeChunk {
                                id: Uuid::new_v4().to_string(),
                                rel_path: rel_path.clone(),
                                location: format!("page {p} (part {part_i})"),
                                content: slice,
                                terms,
                                embedding: None,
                            });
                            added_any = true;
                        }
                    }
                }
                if added_any {
                    file_count += 1;
                }
            }
        } else if let Ok(raw) = std::fs::read_to_string(path) {
            let mut added_any = false;
            for (chunk_i, slice) in chunk_text(&raw, 700, 100) {
                let terms = term_counts(&slice);
                chunks.push(KnowledgeChunk {
                    id: Uuid::new_v4().to_string(),
                    rel_path: rel_path.clone(),
                    location: format!("chunk {chunk_i}"),
                    content: slice,
                    terms,
                    embedding: None,
                });
                added_any = true;
            }
            if added_any {
                file_count += 1;
            }
        }

        if chunks.len() >= 1200 {
            break;
        }
    }

    // Probe optional local loopback Ollama embedding models (bge-m3 / nomic-embed-text)
    let mut embedding_model: Option<String> = None;
    let mut has_dense_embeddings = false;

    if !chunks.is_empty() {
        if let Some(model_name) = detect_local_embed_model(gate).await {
            let max_embed = chunks.len().min(128);
            let mut embedded_any = false;
            for (batch_idx, start_i) in (0..max_embed).step_by(32).enumerate() {
                let end_i = (start_i + 32).min(max_embed);
                let texts: Vec<String> = chunks[start_i..end_i]
                    .iter()
                    .map(|c| c.content.clone())
                    .collect();
                let timeout_ms = if batch_idx == 0 { 6000 } else { 3000 };
                if let Some(vecs) =
                    request_ollama_embeddings(gate, &model_name, &texts, timeout_ms).await
                {
                    for (chunk, vec) in chunks[start_i..end_i].iter_mut().zip(vecs.into_iter()) {
                        chunk.embedding = Some(vec);
                        embedded_any = true;
                    }
                } else {
                    break;
                }
            }
            if embedded_any {
                has_dense_embeddings = true;
                embedding_model = Some(model_name);
            }
        }
    }

    let mut doc_freqs: HashMap<String, usize> = HashMap::new();
    let mut total_terms = 0usize;
    for c in &chunks {
        let len: usize = c.terms.values().sum();
        total_terms += len;
        for term in c.terms.keys() {
            *doc_freqs.entry(term.clone()).or_insert(0) += 1;
        }
    }
    let avg_doc_len = if chunks.is_empty() {
        1.0
    } else {
        (total_terms as f64 / chunks.len() as f64).max(1.0)
    };

    let stats = IndexStats {
        workspace_path: workspace_root.to_string_lossy().to_string(),
        total_files: file_count,
        total_chunks: chunks.len(),
        has_dense_embeddings,
        embedding_model,
        indexed_at: Utc::now(),
    };

    let idx = WorkspaceKnowledgeIndex {
        stats: stats.clone(),
        avg_doc_len,
        doc_freqs,
        chunks,
    };

    let idx_path = index_file_path(workspace_root);
    if let Some(parent) = idx_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = idx_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(&idx)?)?;
    std::fs::rename(&tmp, &idx_path)?;

    Ok(stats)
}

pub async fn search_workspace_knowledge(
    workspace_root: &Path,
    gate: &NetworkGate,
    query: &str,
    top_k: usize,
) -> anyhow::Result<Vec<KnowledgeSearchHit>> {
    search_workspace_knowledge_with_policy(workspace_root, gate, query, top_k, false).await
}

pub async fn search_workspace_knowledge_with_policy(
    workspace_root: &Path,
    gate: &NetworkGate,
    query: &str,
    top_k: usize,
    skip_secret_contents: bool,
) -> anyhow::Result<Vec<KnowledgeSearchHit>> {
    let idx_path = index_file_path(workspace_root);
    if !idx_path.exists() {
        build_workspace_index(workspace_root, gate).await?;
    }
    let raw = std::fs::read_to_string(&idx_path)?;
    let idx: WorkspaceKnowledgeIndex = serde_json::from_str(&raw)?;
    if idx.chunks.is_empty() {
        return Ok(Vec::new());
    }

    let query_terms = tokenize(query);
    let lower_query = query.trim().to_ascii_lowercase();
    let n_docs = idx.chunks.len() as f64;
    let k1 = 1.5;
    let b = 0.75;

    let query_vec = if idx.stats.has_dense_embeddings {
        if let Some(ref model_name) = idx.stats.embedding_model {
            request_ollama_embeddings(gate, model_name, &[query.to_string()], 3000)
                .await
                .and_then(|mut v| v.pop())
        } else {
            None
        }
    } else {
        None
    };

    let mut per_chunk: Vec<(usize, f64, f64)> = idx
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            if skip_secret_contents && crate::risk::is_sensitive_secret_path(&chunk.rel_path) {
                return (i, 0.0, 0.0);
            }
            let doc_len: f64 = chunk.terms.values().sum::<usize>().max(1) as f64;
            let mut bm25 = 0.0;
            for qt in &query_terms {
                let tf = *chunk.terms.get(qt).unwrap_or(&0) as f64;
                if tf > 0.0 {
                    let df = *idx.doc_freqs.get(qt).unwrap_or(&1) as f64;
                    let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.1);
                    let denom = tf + k1 * (1.0 - b + b * (doc_len / idx.avg_doc_len));
                    bm25 += idf * (tf * (k1 + 1.0) / denom);
                }
            }
            if !lower_query.is_empty()
                && chunk.content.to_ascii_lowercase().contains(&lower_query)
            {
                bm25 += 2.5;
            }
            let cos = match (&query_vec, &chunk.embedding) {
                (Some(qv), Some(cv)) => cosine_similarity(qv, cv).max(0.0),
                _ => 0.0,
            };
            (i, bm25, cos)
        })
        .collect();

    // Rank separately for Reciprocal Rank Fusion (RRF) when dense embeddings exist
    let mut rrf_scores = vec![0.0f64; idx.chunks.len()];
    per_chunk.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (rank, &(chunk_idx, bm25, _)) in per_chunk.iter().enumerate() {
        if bm25 > 0.0 {
            if query_vec.is_some() {
                rrf_scores[chunk_idx] += 100.0 / (60.0 + (rank as f64) + 1.0);
            } else {
                rrf_scores[chunk_idx] = bm25;
            }
        }
    }
    if query_vec.is_some() {
        per_chunk.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        for (rank, &(chunk_idx, _, cos)) in per_chunk.iter().enumerate() {
            if cos > 0.12 {
                rrf_scores[chunk_idx] += 100.0 / (60.0 + (rank as f64) + 1.0);
            }
        }
    }

    let mut scored: Vec<KnowledgeSearchHit> = idx
        .chunks
        .iter()
        .enumerate()
        .filter_map(|(i, chunk)| {
            let total_score = rrf_scores[i];
            if total_score <= 0.0 {
                return None;
            }
            let snippet = if chunk.content.len() > 420 {
                let mut end = 420;
                while !chunk.content.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &chunk.content[..end])
            } else {
                chunk.content.clone()
            };

            Some(KnowledgeSearchHit {
                rel_path: chunk.rel_path.clone(),
                location: chunk.location.clone(),
                score: (total_score * 100.0).round() / 100.0,
                snippet,
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k.max(1));
    Ok(scored)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = x as f64;
        let yf = y as f64;
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

async fn detect_local_embed_model(gate: &NetworkGate) -> Option<String> {
    let req = gate
        .request(Method::GET, "http://127.0.0.1:11434/api/tags", "rag_embed_probe")
        .ok()?;
    let resp = tokio::time::timeout(std::time::Duration::from_millis(400), req.send())
        .await
        .ok()?
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    let arr = val["models"].as_array()?;
    for pref in ["bge-m3", "nomic-embed-text"] {
        for m in arr {
            if let Some(name) = m["name"].as_str() {
                if name.to_ascii_lowercase().contains(pref) {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

async fn request_ollama_embeddings(
    gate: &NetworkGate,
    model_name: &str,
    inputs: &[String],
    timeout_ms: u64,
) -> Option<Vec<Vec<f32>>> {
    let req = gate
        .request(
            Method::POST,
            "http://127.0.0.1:11434/api/embed",
            "rag_local_embed",
        )
        .ok()?
        .json(&json!({
            "model": model_name,
            "input": inputs,
        }));
    let resp = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), req.send())
        .await
        .ok()?
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    let arr = val["embeddings"].as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let vec: Vec<f32> = item
            .as_array()?
            .iter()
            .filter_map(|v| v.as_f64().map(|x| x as f32))
            .collect();
        out.push(vec);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_net::PrivacyMode;

    #[test]
    fn offline_rag_indexes_and_finds_relevant_chunks() {
        let dir = std::env::temp_dir().join(format!("bumbo_rag_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("notes.md"),
            "# Post-Quantum Cryptography\nML-KEM-768 provides quantum-resistant key encapsulation in OpenSSL 3.5 benchmarks.\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("other.txt"),
            "Unrelated grocery shopping list: apples and bread.\n",
        )
        .unwrap();

        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let stats = build_workspace_index(&dir, &gate).await.unwrap();
            assert_eq!(stats.total_files, 2);
            assert!(stats.total_chunks >= 2);

            let hits = search_workspace_knowledge(&dir, &gate, "ML-KEM OpenSSL", 3)
                .await
                .unwrap();
            assert!(!hits.is_empty());
            assert_eq!(hits[0].rel_path, "notes.md");
            assert_eq!(gate.audit_summary().external_requests_allowed, 0);
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_rag_excludes_secret_files_and_honors_cloud_guard() {
        let dir = std::env::temp_dir().join(format!("bumbo_rag_secret_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "Public architecture documentation.\n").unwrap();
        std::fs::write(
            dir.join("credentials.json"),
            "{\"secret_token\": \"super_secret_leaked_token_123\"}\n",
        )
        .unwrap();

        let gate = NetworkGate::new(PrivacyMode::StrictLocal).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let stats = build_workspace_index(&dir, &gate).await.unwrap();
            assert_eq!(stats.total_files, 1);

            let hits = search_workspace_knowledge_with_policy(
                &dir,
                &gate,
                "super_secret_leaked_token_123",
                5,
                true,
            )
            .await
            .unwrap();
            assert!(hits.is_empty());

            // Also test filtering on a pre-existing index containing a secret chunk
            let idx_path = index_file_path(&dir);
            let mut idx: WorkspaceKnowledgeIndex =
                serde_json::from_str(&std::fs::read_to_string(&idx_path).unwrap()).unwrap();
            idx.chunks.push(KnowledgeChunk {
                id: Uuid::new_v4().to_string(),
                rel_path: "secrets.toml".into(),
                location: "chunk 1".into(),
                content: "api_key = super_secret_leaked_token_123".into(),
                terms: term_counts("api_key super_secret_leaked_token_123"),
                embedding: None,
            });
            std::fs::write(&idx_path, serde_json::to_string(&idx).unwrap()).unwrap();

            let guarded = search_workspace_knowledge_with_policy(
                &dir,
                &gate,
                "super_secret_leaked_token_123",
                5,
                true,
            )
            .await
            .unwrap();
            assert!(guarded.is_empty());
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}
