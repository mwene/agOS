use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const TOKEN_COST_LOCAL: u64 = 2;
const TOKEN_COST_CLOUD: u64 = 5;

const DRIVER_LOCAL: u32 = 0;
const DRIVER_CLOUD: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UMIResult {
    pub output: Vec<String>,
    pub tokens_used: u64,
    pub model: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverMetrics {
    pub name: String,
    pub capability_level: u32,
    pub calls: u64,
    pub total_tokens: u64,
}

struct DriverState {
    name: &'static str,
    capability_level: u32,
    calls: AtomicU64,
    total_tokens: AtomicU64,
    token_cost: u64,
    prefix: &'static str,
    embed_modulus: u64,
    max_output: usize,
}

static LOCAL_DRIVER: DriverState = DriverState {
    name: "LocalEngine-v0",
    capability_level: 2,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: TOKEN_COST_LOCAL,
    prefix: "reasoned",
    embed_modulus: 10000,
    max_output: 10,
};

static CLOUD_DRIVER: DriverState = DriverState {
    name: "CloudGPT-HighReason",
    capability_level: 5,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: TOKEN_COST_CLOUD,
    prefix: "deep_reasoned",
    embed_modulus: 100000,
    max_output: 20,
};

pub struct UnifiedModelInterface {
    active: AtomicU32,
}

impl UnifiedModelInterface {
    pub fn new() -> Self {
        UnifiedModelInterface {
            active: AtomicU32::new(DRIVER_LOCAL),
        }
    }

    fn driver(&self) -> &'static DriverState {
        match self.active.load(Ordering::Relaxed) {
            DRIVER_CLOUD => &CLOUD_DRIVER,
            _ => &LOCAL_DRIVER,
        }
    }

    pub fn active_name(&self) -> &'static str {
        self.driver().name
    }

    pub fn active_capability(&self) -> u32 {
        self.driver().capability_level
    }

    pub fn hot_swap(&self, driver_name: &str) -> Result<(), String> {
        match driver_name {
            "LocalEngine-v0" => {
                self.active.store(DRIVER_LOCAL, Ordering::Relaxed);
                Ok(())
            }
            "CloudGPT-HighReason" => {
                self.active.store(DRIVER_CLOUD, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(format!("Driver '{}' not found. Available: LocalEngine-v0, CloudGPT-HighReason", driver_name)),
        }
    }

    pub async fn reason(&self, prompt_vector: &[String], _schema: &Value) -> UMIResult {
        let d = self.driver();
        d.calls.fetch_add(1, Ordering::Relaxed);
        let tokens = (prompt_vector.len() as u64).max(1) * d.token_cost;
        d.total_tokens.fetch_add(tokens, Ordering::Relaxed);
        let output: Vec<String> = prompt_vector
            .iter()
            .take(d.max_output)
            .map(|s| format!("{}:{}", d.prefix, s))
            .collect();
        UMIResult {
            output,
            tokens_used: tokens,
            model: d.name.to_string(),
            mode: "SYS_REASON".to_string(),
        }
    }

    pub fn embed(&self, data: &[String]) -> Vec<f64> {
        let d = self.driver();
        d.calls.fetch_add(1, Ordering::Relaxed);
        use std::hash::{Hash, Hasher};
        data.iter()
            .map(|s| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                s.hash(&mut h);
                (h.finish() % d.embed_modulus) as f64 / d.embed_modulus as f64
            })
            .collect()
    }

    pub fn compress(&self, data: &[String]) -> Value {
        let d = self.driver();
        d.calls.fetch_add(1, Ordering::Relaxed);
        serde_json::json!({
            "original_length": data.len(),
            "driver": d.name,
            "compressed_at": chrono::Utc::now().to_rfc3339(),
        })
    }

    pub fn get_metrics(&self) -> Vec<DriverMetrics> {
        let local = DriverMetrics {
            name: LOCAL_DRIVER.name.to_string(),
            capability_level: LOCAL_DRIVER.capability_level,
            calls: LOCAL_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: LOCAL_DRIVER.total_tokens.load(Ordering::Relaxed),
        };
        let cloud = DriverMetrics {
            name: CLOUD_DRIVER.name.to_string(),
            capability_level: CLOUD_DRIVER.capability_level,
            calls: CLOUD_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: CLOUD_DRIVER.total_tokens.load(Ordering::Relaxed),
        };
        vec![local, cloud]
    }
}
