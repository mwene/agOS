use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub compressed_entries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicConfig {
    pub compression_interval_secs: u64,
    pub max_entries_before_compress: usize,
    pub entry_ttl_secs: u64,
}

impl Default for EpisodicConfig {
    fn default() -> Self {
        EpisodicConfig {
            compression_interval_secs: 60,
            max_entries_before_compress: 100,
            entry_ttl_secs: 300,
        }
    }
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
    compressed_entries: AtomicU64,
    episodic_config: EpisodicConfig,
}

impl Default for HierarchicalMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HierarchicalMemoryManager {
    pub fn new() -> Self {
        HierarchicalMemoryManager {
            l1: dashmap::DashMap::new(),
            l2: dashmap::DashMap::new(),
            vector_ram: std::sync::RwLock::new(Vec::new()),
            episodic_ledger: std::sync::RwLock::new(Vec::new()),
            compression_factor: 0.3,
            compressed_entries: AtomicU64::new(0),
            episodic_config: EpisodicConfig::default(),
        }
    }

    pub fn with_config(config: EpisodicConfig) -> Self {
        HierarchicalMemoryManager {
            l1: dashmap::DashMap::new(),
            l2: dashmap::DashMap::new(),
            vector_ram: std::sync::RwLock::new(Vec::new()),
            episodic_ledger: std::sync::RwLock::new(Vec::new()),
            compression_factor: 0.3,
            compressed_entries: AtomicU64::new(0),
            episodic_config: config,
        }
    }

    pub fn episodic_config(&self) -> EpisodicConfig {
        self.episodic_config.clone()
    }

    pub fn set_episodic_config(&mut self, config: EpisodicConfig) {
        self.episodic_config = config;
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
            .or_default()
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

    pub fn compress_episodic(&self) -> usize {
        let cutoff = chrono::Utc::now()
            - chrono::Duration::seconds(self.episodic_config.entry_ttl_secs as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let mut ledger = self.episodic_ledger.write().unwrap();
        let (fresh, stale): (Vec<EpisodicEntry>, Vec<EpisodicEntry>) = ledger
            .iter()
            .cloned()
            .partition(|e| e.timestamp > cutoff_str);

        if stale.is_empty() {
            return 0;
        }

        let mut groups: std::collections::HashMap<(String, String), Vec<EpisodicEntry>> =
            std::collections::HashMap::new();
        for entry in &stale {
            groups
                .entry((entry.pid.clone(), entry.event_type.clone()))
                .or_default()
                .push(entry.clone());
        }

        for ((pid, event_type), entries) in &groups {
            let errors: Vec<&str> = entries
                .iter()
                .filter_map(|e| e.error.as_deref())
                .collect();
            let summary = serde_json::json!({
                "event_type": event_type,
                "count": entries.len(),
                "compressed_at": chrono::Utc::now().to_rfc3339(),
                "time_range": {
                    "oldest": entries.first().map(|e| &e.timestamp),
                    "newest": entries.last().map(|e| &e.timestamp),
                },
                "error_count": errors.len(),
            });
            self.write_l2(pid, &format!("episodic_{}", event_type), summary);
        }

        let compressed_count = stale.len();
        *ledger = fresh;

        self.compressed_entries
            .fetch_add(compressed_count as u64, Ordering::Relaxed);

        compressed_count
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
        let compressed_entries = self.compressed_entries.load(Ordering::Relaxed);
        MemoryStats {
            l1_size,
            l2_entries,
            vector_ram_entries,
            episodic_entries,
            compressed_entries,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_store_and_page_in() {
        let mem = HierarchicalMemoryManager::new();
        mem.store_vector(vec![1.0, 0.0], serde_json::json!({"label": "a"}));
        mem.store_vector(vec![0.0, 1.0], serde_json::json!({"label": "b"}));
        mem.store_vector(vec![0.9, 0.1], serde_json::json!({"label": "c"}));

        let results = mem.page_in(&[1.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].metadata["label"], "a");
    }

    #[test]
    fn test_compress_episodic_no_stale() {
        let mem = HierarchicalMemoryManager::new();
        mem.log_episodic("proc-1", "TEST", serde_json::json!({"msg": "fresh"}), None);
        let count = mem.compress_episodic();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_compress_episodic_with_stale() {
        let mem = HierarchicalMemoryManager::with_config(EpisodicConfig {
            compression_interval_secs: 60,
            max_entries_before_compress: 10,
            entry_ttl_secs: 0,
        });

        mem.log_episodic("proc-1", "EVENT_A", serde_json::json!({"seq": 1}), None);
        mem.log_episodic("proc-1", "EVENT_A", serde_json::json!({"seq": 2}), None);
        mem.log_episodic("proc-2", "EVENT_B", serde_json::json!({"seq": 3}), None);

        let count = mem.compress_episodic();
        assert_eq!(count, 3);

        let stats = mem.get_stats();
        assert_eq!(stats.episodic_entries, 0);
        assert_eq!(stats.compressed_entries, 3);

        let l2 = mem.read_l2("proc-1");
        assert!(!l2.is_empty());
    }

    #[test]
    fn test_compress_episodic_partial() {
        let mem = HierarchicalMemoryManager::with_config(EpisodicConfig {
            compression_interval_secs: 60,
            max_entries_before_compress: 10,
            entry_ttl_secs: 3600,
        });

        let old_entry = EpisodicEntry {
            pid: "proc-1".to_string(),
            event_type: "OLD".to_string(),
            payload: serde_json::json!({"v": 1}),
            error: None,
            timestamp: "2020-01-01T00:00:00+00:00".to_string(),
            event_id: Uuid::new_v4().to_string(),
        };
        {
            let mut ledger = mem.episodic_ledger.write().unwrap();
            ledger.push(old_entry);
        }

        let count = mem.compress_episodic();
        assert_eq!(count, 1);

        mem.log_episodic("proc-1", "NEW", serde_json::json!({"v": 2}), None);

        let count2 = mem.compress_episodic();
        assert_eq!(count2, 0);

        let stats = mem.get_stats();
        assert_eq!(stats.episodic_entries, 1);
        assert_eq!(stats.compressed_entries, 1);
    }

    #[test]
    fn test_episodic_config_defaults() {
        let cfg = EpisodicConfig::default();
        assert_eq!(cfg.compression_interval_secs, 60);
        assert_eq!(cfg.entry_ttl_secs, 300);
    }

    #[test]
    fn test_memory_stats_includes_compressed() {
        let mem = HierarchicalMemoryManager::new();
        let stats = mem.get_stats();
        assert_eq!(stats.compressed_entries, 0);
    }
}
