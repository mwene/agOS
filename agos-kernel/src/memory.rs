use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub vector: Vec<f64>,
    pub metadata: Value,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicEntry {
    pub pid: String,
    pub event_type: String,
    pub payload: Value,
    pub error: Option<String>,
    pub timestamp: String,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub l1_size: usize,
    pub l2_entries: usize,
    pub vector_ram_entries: usize,
    pub episodic_entries: usize,
}

pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f64 = a.iter().map(|x| x * x).sum();
    let nb: f64 = b.iter().map(|y| y * y).sum();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

pub struct HierarchicalMemoryManager {
    l1: dashmap::DashMap<String, Vec<Value>>,
    l2: dashmap::DashMap<String, dashmap::DashMap<String, Value>>,
    vector_ram: std::sync::RwLock<Vec<VectorEntry>>,
    episodic_ledger: std::sync::RwLock<Vec<EpisodicEntry>>,
    compression_factor: f64,
}

impl HierarchicalMemoryManager {
    pub fn new() -> Self {
        HierarchicalMemoryManager {
            l1: dashmap::DashMap::new(),
            l2: dashmap::DashMap::new(),
            vector_ram: std::sync::RwLock::new(Vec::new()),
            episodic_ledger: std::sync::RwLock::new(Vec::new()),
            compression_factor: 0.3,
        }
    }

    pub fn write_l1(&self, pid: &str, data: Vec<Value>) {
        self.l1.insert(pid.to_string(), data);
    }

    pub fn append_l1(&self, pid: &str, item: Value) {
        self.l1.entry(pid.to_string()).or_default().push(item);
    }

    pub fn read_l1(&self, pid: &str) -> Vec<Value> {
        self.l1.get(pid).map(|v| v.clone()).unwrap_or_default()
    }

    pub fn write_l2(&self, pid: &str, key: &str, compressed: Value) {
        self.l2
            .entry(pid.to_string())
            .or_insert_with(dashmap::DashMap::new)
            .insert(key.to_string(), compressed);
    }

    pub fn read_l2(&self, pid: &str) -> Vec<(String, Value)> {
        self.l2
            .get(pid)
            .map(|map| {
                map.iter()
                    .map(|e| (e.key().clone(), e.value().clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn store_vector(&self, vector: Vec<f64>, metadata: Value) {
        let mut ram = self.vector_ram.write().unwrap();
        ram.push(VectorEntry {
            vector,
            metadata,
            ts: Utc::now().to_rfc3339(),
        });
    }

    pub fn page_in(&self, query_vector: &[f64], top_k: usize) -> Vec<VectorEntry> {
        let ram = self.vector_ram.read().unwrap();
        let mut scored: Vec<(f64, &VectorEntry)> = ram
            .iter()
            .map(|e| (cosine_similarity(query_vector, &e.vector), e))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored.into_iter().map(|(_, e)| e.clone()).collect()
    }

    pub fn compress_to_l2(&self, pid: &str) -> Value {
        let data = self.read_l1(pid);
        let compressed = serde_json::json!({
            "summary_hash": format!("{:x}", hash_data(&data)),
            "vector_count": data.len(),
            "compressed_at": Utc::now().to_rfc3339(),
            "compression_ratio": self.compression_factor,
        });
        let key = format!("step_{}", Utc::now().timestamp());
        self.write_l2(pid, &key, compressed.clone());
        compressed
    }

    pub fn flush_l1(&self, pid: &str) {
        self.l1.remove(pid);
    }

    pub fn log_episodic(
        &self,
        pid: &str,
        event_type: &str,
        payload: Value,
        error: Option<String>,
    ) -> EpisodicEntry {
        let entry = EpisodicEntry {
            pid: pid.to_string(),
            event_type: event_type.to_string(),
            payload,
            error,
            timestamp: Utc::now().to_rfc3339(),
            event_id: Uuid::new_v4().to_string(),
        };
        let mut ledger = self.episodic_ledger.write().unwrap();
        ledger.push(entry.clone());
        entry
    }

    pub fn get_episodic_log(&self, pid: Option<&str>, limit: usize) -> Vec<EpisodicEntry> {
        let ledger = self.episodic_ledger.read().unwrap();
        let iter: Vec<EpisodicEntry> = match pid {
            Some(pid) => ledger
                .iter()
                .filter(|e| e.pid == pid)
                .cloned()
                .collect(),
            None => ledger.clone(),
        };
        iter.into_iter().rev().take(limit).rev().collect()
    }

    pub fn get_stats(&self) -> MemoryStats {
        let l1_size: usize = self.l1.iter().map(|e| e.value().len()).sum();
        let l2_entries: usize = self.l2.iter().map(|e| e.value().len()).sum();
        let vector_ram_entries = self.vector_ram.read().unwrap().len();
        let episodic_entries = self.episodic_ledger.read().unwrap().len();
        MemoryStats {
            l1_size,
            l2_entries,
            vector_ram_entries,
            episodic_entries,
        }
    }
}

fn hash_data(data: &[Value]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for item in data {
        item.to_string().hash(&mut hasher);
    }
    hasher.finish()
}
