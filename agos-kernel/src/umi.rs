use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverInfo {
    pub name: String,
    pub capability_level: u32,
    pub token_cost: u64,
    pub prefix: String,
    pub embed_modulus: u64,
    pub max_output: usize,
    pub calls: u64,
    pub total_tokens: u64,
    pub driver_type: String,
}

struct RuntimeDriverState {
    name: String,
    capability_level: u32,
    token_cost: u64,
    prefix: String,
    embed_modulus: u64,
    max_output: usize,
    calls: u64,
    total_tokens: u64,
    config: HashMap<String, String>,
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
    runtime_drivers: Mutex<HashMap<String, RuntimeDriverState>>,
}

impl Default for UnifiedModelInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl UnifiedModelInterface {
    pub fn new() -> Self {
        UnifiedModelInterface {
            active: AtomicU32::new(DRIVER_LOCAL),
            runtime_drivers: Mutex::new(HashMap::new()),
        }
    }

    pub fn active_name(&self) -> String {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_CLOUD {
            return self.with_static_driver(|d| d.name.to_string());
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().map(|d| d.name.clone()).unwrap_or_else(|| LOCAL_DRIVER.name.to_string())
    }

    pub fn active_capability(&self) -> u32 {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_CLOUD {
            return self.with_static_driver(|d| d.capability_level);
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().map(|d| d.capability_level).unwrap_or(LOCAL_DRIVER.capability_level)
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
            _ => {
                let drivers = self.runtime_drivers.lock().unwrap();
                if drivers.contains_key(driver_name) {
                    self.active.store(DRIVER_CLOUD + 1, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(format!(
                        "Driver '{}' not found. Available: LocalEngine-v0, CloudGPT-HighReason, and {} runtime driver(s)",
                        driver_name,
                        drivers.len()
                    ))
                }
            }
        }
    }

    pub fn list_drivers(&self) -> Vec<DriverInfo> {
        let mut list = Vec::new();
        for d in [&LOCAL_DRIVER, &CLOUD_DRIVER].iter() {
            list.push(DriverInfo {
                name: d.name.to_string(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                prefix: d.prefix.to_string(),
                embed_modulus: d.embed_modulus,
                max_output: d.max_output,
                calls: d.calls.load(Ordering::Relaxed),
                total_tokens: d.total_tokens.load(Ordering::Relaxed),
                driver_type: "builtin".into(),
            });
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        for d in drivers.values() {
            list.push(DriverInfo {
                name: d.name.clone(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                prefix: d.prefix.clone(),
                embed_modulus: d.embed_modulus,
                max_output: d.max_output,
                calls: d.calls,
                total_tokens: d.total_tokens,
                driver_type: "runtime".into(),
            });
        }
        list
    }

    pub fn get_driver(&self, name: &str) -> Option<DriverInfo> {
        if let Some(d) = [&LOCAL_DRIVER, &CLOUD_DRIVER].iter().find(|d| d.name == name) {
            return Some(DriverInfo {
                name: d.name.to_string(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                prefix: d.prefix.to_string(),
                embed_modulus: d.embed_modulus,
                max_output: d.max_output,
                calls: d.calls.load(Ordering::Relaxed),
                total_tokens: d.total_tokens.load(Ordering::Relaxed),
                driver_type: "builtin".into(),
            });
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.get(name).map(|d| DriverInfo {
            name: d.name.clone(),
            capability_level: d.capability_level,
            token_cost: d.token_cost,
            prefix: d.prefix.clone(),
            embed_modulus: d.embed_modulus,
            max_output: d.max_output,
            calls: d.calls,
            total_tokens: d.total_tokens,
            driver_type: "runtime".into(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_driver(
        &self,
        name: &str,
        capability_level: u32,
        token_cost: u64,
        prefix: &str,
        embed_modulus: u64,
        max_output: usize,
        config: HashMap<String, String>,
    ) -> Result<(), String> {
        if name.is_empty() {
            return Err("Driver name cannot be empty".into());
        }
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return Err(format!("Cannot register driver with reserved name '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        if drivers.contains_key(name) {
            return Err(format!("Driver '{}' already registered", name));
        }
        drivers.insert(
            name.to_string(),
            RuntimeDriverState {
                name: name.to_string(),
                capability_level,
                token_cost,
                prefix: prefix.to_string(),
                embed_modulus,
                max_output,
                calls: 0,
                total_tokens: 0,
                config,
            },
        );
        Ok(())
    }

    pub fn get_driver_config(&self, name: &str) -> Option<HashMap<String, String>> {
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return None;
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.get(name).map(|d| d.config.clone())
    }

    pub fn update_driver_config(&self, name: &str, config: HashMap<String, String>) -> Result<(), String> {
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return Err(format!("Cannot configure built-in driver '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        let d = drivers.get_mut(name).ok_or_else(|| format!("Driver '{}' not found", name))?;
        d.config = config;
        Ok(())
    }

    pub fn unregister_driver(&self, name: &str) -> Result<(), String> {
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return Err(format!("Cannot unregister built-in driver '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        drivers.remove(name).ok_or_else(|| format!("Driver '{}' not found", name))?;
        Ok(())
    }

    fn with_static_driver<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&'static DriverState) -> R,
    {
        match self.active.load(Ordering::Relaxed) {
            DRIVER_CLOUD => f(&CLOUD_DRIVER),
            _ => f(&LOCAL_DRIVER),
        }
    }

    pub async fn reason(&self, prompt_vector: &[String], _schema: &Value) -> UMIResult {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_CLOUD {
            return self.with_static_driver(|d| {
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
            });
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        let d = drivers.values_mut().next();
        if let Some(d) = d {
            d.calls += 1;
            let tokens = (prompt_vector.len() as u64).max(1) * d.token_cost;
            d.total_tokens += tokens;
            let output: Vec<String> = prompt_vector
                .iter()
                .take(d.max_output)
                .map(|s| format!("{}:{}", d.prefix, s))
                .collect();
            UMIResult {
                output,
                tokens_used: tokens,
                model: d.name.clone(),
                mode: "SYS_REASON".to_string(),
            }
        } else {
            // Fallback to local driver
            let d = &LOCAL_DRIVER;
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
    }

    pub fn embed(&self, data: &[String]) -> Vec<f64> {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_CLOUD {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                use std::hash::{Hash, Hasher};
                data.iter()
                    .map(|s| {
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        s.hash(&mut h);
                        (h.finish() % d.embed_modulus) as f64 / d.embed_modulus as f64
                    })
                    .collect()
            });
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        if let Some(d) = drivers.values_mut().next() {
            d.calls += 1;
            use std::hash::{Hash, Hasher};
            data.iter()
                .map(|s| {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    s.hash(&mut h);
                    (h.finish() % d.embed_modulus) as f64 / d.embed_modulus as f64
                })
                .collect()
        } else {
            let d = &LOCAL_DRIVER;
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
    }

    pub fn compress(&self, data: &[String]) -> Value {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_CLOUD {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                serde_json::json!({
                    "original_length": data.len(),
                    "driver": d.name,
                    "compressed_at": chrono::Utc::now().to_rfc3339(),
                })
            });
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        if let Some(d) = drivers.values_mut().next() {
            d.calls += 1;
            serde_json::json!({
                "original_length": data.len(),
                "driver": d.name,
                "compressed_at": chrono::Utc::now().to_rfc3339(),
            })
        } else {
            let d = &LOCAL_DRIVER;
            d.calls.fetch_add(1, Ordering::Relaxed);
            serde_json::json!({
                "original_length": data.len(),
                "driver": d.name,
                "compressed_at": chrono::Utc::now().to_rfc3339(),
            })
        }
    }

    pub fn get_metrics(&self) -> Vec<DriverMetrics> {
        let mut metrics = Vec::new();
        metrics.push(DriverMetrics {
            name: LOCAL_DRIVER.name.to_string(),
            capability_level: LOCAL_DRIVER.capability_level,
            calls: LOCAL_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: LOCAL_DRIVER.total_tokens.load(Ordering::Relaxed),
        });
        metrics.push(DriverMetrics {
            name: CLOUD_DRIVER.name.to_string(),
            capability_level: CLOUD_DRIVER.capability_level,
            calls: CLOUD_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: CLOUD_DRIVER.total_tokens.load(Ordering::Relaxed),
        });
        let drivers = self.runtime_drivers.lock().unwrap();
        for d in drivers.values() {
            metrics.push(DriverMetrics {
                name: d.name.clone(),
                capability_level: d.capability_level,
                calls: d.calls,
                total_tokens: d.total_tokens,
            });
        }
        metrics
    }
}
