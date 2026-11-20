use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::llm::{LlmConfig, LlmDriver, DRIVER_NAME as LIVE_DRIVER_NAME};

const TOKEN_COST_LOCAL: u64 = 2;
const TOKEN_COST_CLOUD: u64 = 5;

const DRIVER_LOCAL: u32 = 0;
const DRIVER_CLOUD: u32 = 1;
/// Marker used for the first runtime-registered driver.
const DRIVER_RUNTIME: u32 = 2;
/// Sentinel for the live OpenAI-compatible driver.
const DRIVER_LIVE: u32 = 100;

const LIVE_DRIVER_CAPABILITY: u32 = 8;
const LIVE_EMBED_MODULUS: u64 = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UMIResult {
    pub output: Vec<String>,
    pub tokens_used: u64,
    pub model: String,
    pub mode: String,
    /// Present only when a live driver was selected and the call failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

/// Which execution path a driver-name resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveDriver {
    Local,
    Cloud,
    Live,
    Runtime,
}

pub struct UnifiedModelInterface {
    active: AtomicU32,
    runtime_drivers: Mutex<HashMap<String, RuntimeDriverState>>,
    live: Option<LlmDriver>,
}

impl Default for UnifiedModelInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl UnifiedModelInterface {
    /// Build the UMI, enabling the live OpenAI-compatible driver when the
    /// environment configures one. Without configuration the kernel stays on
    /// the deterministic built-in drivers.
    pub fn new() -> Self {
        let live = match LlmConfig::from_env() {
            Some(config) => match LlmDriver::new(config) {
                Ok(driver) => {
                    let snapshot = driver.config_snapshot();
                    tracing::info!(
                        model = %snapshot.model,
                        endpoint = %snapshot.endpoint(),
                        "UMI live driver enabled"
                    );
                    Some(driver)
                }
                Err(err) => {
                    tracing::error!("UMI live driver disabled: {}", err);
                    None
                }
            },
            None => None,
        };

        UnifiedModelInterface {
            active: AtomicU32::new(DRIVER_LOCAL),
            runtime_drivers: Mutex::new(HashMap::new()),
            live,
        }
    }

    /// Build a UMI with a live driver and make it active. Used by tests and by
    /// embedders that construct configuration in code instead of the process
    /// environment.
    pub fn with_live_config(config: LlmConfig) -> Result<Self, String> {
        let driver = LlmDriver::new(config)?;
        Ok(UnifiedModelInterface {
            active: AtomicU32::new(DRIVER_LIVE),
            runtime_drivers: Mutex::new(HashMap::new()),
            live: Some(driver),
        })
    }

    pub fn has_live_driver(&self) -> bool {
        self.live.is_some()
    }

    fn active_kind(&self) -> ActiveDriver {
        match self.active.load(Ordering::Relaxed) {
            DRIVER_LOCAL => ActiveDriver::Local,
            DRIVER_CLOUD => ActiveDriver::Cloud,
            DRIVER_LIVE => ActiveDriver::Live,
            _ => ActiveDriver::Runtime,
        }
    }

    pub fn active_name(&self) -> String {
        match self.active_kind() {
            ActiveDriver::Local => LOCAL_DRIVER.name.to_string(),
            ActiveDriver::Cloud => CLOUD_DRIVER.name.to_string(),
            ActiveDriver::Live => LIVE_DRIVER_NAME.to_string(),
            ActiveDriver::Runtime => {
                let drivers = self.runtime_drivers.lock().unwrap();
                drivers
                    .values()
                    .next()
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| LOCAL_DRIVER.name.to_string())
            }
        }
    }

    pub fn active_capability(&self) -> u32 {
        match self.active_kind() {
            ActiveDriver::Local => LOCAL_DRIVER.capability_level,
            ActiveDriver::Cloud => CLOUD_DRIVER.capability_level,
            ActiveDriver::Live => LIVE_DRIVER_CAPABILITY,
            ActiveDriver::Runtime => {
                let drivers = self.runtime_drivers.lock().unwrap();
                drivers
                    .values()
                    .next()
                    .map(|d| d.capability_level)
                    .unwrap_or(LOCAL_DRIVER.capability_level)
            }
        }
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
            LIVE_DRIVER_NAME => match &self.live {
                Some(_) => {
                    self.active.store(DRIVER_LIVE, Ordering::Relaxed);
                    Ok(())
                }
                None => Err(
                    "Live driver 'OpenAICompatible' is not configured. Set AGOS_LLM_BASE_URL and/or \
                     AGOS_LLM_API_KEY (see UMI_DRIVER_GUIDE.md) and restart the kernel."
                        .into(),
                ),
            },
            _ => {
                let drivers = self.runtime_drivers.lock().unwrap();
                if drivers.contains_key(driver_name) {
                    self.active.store(DRIVER_RUNTIME, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(format!(
                        "Driver '{}' not found. Available: LocalEngine-v0, CloudGPT-HighReason, {}, and {} runtime driver(s)",
                        driver_name,
                        LIVE_DRIVER_NAME,
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
        if let Some(live) = &self.live {
            list.push(DriverInfo {
                name: LIVE_DRIVER_NAME.to_string(),
                capability_level: LIVE_DRIVER_CAPABILITY,
                token_cost: 0,
                prefix: live.config_snapshot().model,
                embed_modulus: LIVE_EMBED_MODULUS,
                // Not used for live drivers: responses are not truncated.
                max_output: 0,
                calls: live.calls(),
                total_tokens: live.total_tokens(),
                driver_type: "live".into(),
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
        if let Some(d) = [&LOCAL_DRIVER, &CLOUD_DRIVER]
            .iter()
            .find(|d| d.name == name)
        {
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
        if name == LIVE_DRIVER_NAME {
            return self.live.as_ref().map(|live| DriverInfo {
                name: LIVE_DRIVER_NAME.to_string(),
                capability_level: LIVE_DRIVER_CAPABILITY,
                token_cost: 0,
                prefix: live.config_snapshot().model,
                embed_modulus: LIVE_EMBED_MODULUS,
                max_output: 0,
                calls: live.calls(),
                total_tokens: live.total_tokens(),
                driver_type: "live".into(),
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
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" || name == LIVE_DRIVER_NAME {
            return Err(format!(
                "Cannot register driver with reserved name '{}'",
                name
            ));
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
        if name == LIVE_DRIVER_NAME {
            return self.live.as_ref().map(|live| live.redacted_config());
        }
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return None;
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.get(name).map(|d| d.config.clone())
    }

    pub fn update_driver_config(
        &self,
        name: &str,
        config: HashMap<String, String>,
    ) -> Result<(), String> {
        if name == LIVE_DRIVER_NAME {
            let live = self
                .live
                .as_ref()
                .ok_or_else(|| "Live driver 'OpenAICompatible' is not configured".to_string())?;
            return live.update_config(config);
        }
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" {
            return Err(format!("Cannot configure built-in driver '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        let d = drivers
            .get_mut(name)
            .ok_or_else(|| format!("Driver '{}' not found", name))?;
        d.config = config;
        Ok(())
    }

    pub fn unregister_driver(&self, name: &str) -> Result<(), String> {
        if name == "LocalEngine-v0" || name == "CloudGPT-HighReason" || name == LIVE_DRIVER_NAME {
            return Err(format!("Cannot unregister built-in driver '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        drivers
            .remove(name)
            .ok_or_else(|| format!("Driver '{}' not found", name))?;
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

    fn stub_reason(driver: &'static DriverState, prompt_vector: &[String]) -> UMIResult {
        driver.calls.fetch_add(1, Ordering::Relaxed);
        let tokens = (prompt_vector.len() as u64).max(1) * driver.token_cost;
        driver.total_tokens.fetch_add(tokens, Ordering::Relaxed);
        let output: Vec<String> = prompt_vector
            .iter()
            .take(driver.max_output)
            .map(|s| format!("{}:{}", driver.prefix, s))
            .collect();
        UMIResult {
            output,
            tokens_used: tokens,
            model: driver.name.to_string(),
            mode: "SYS_REASON".to_string(),
            error: None,
        }
    }

    pub async fn reason(&self, prompt_vector: &[String], _schema: &Value) -> UMIResult {
        match self.active_kind() {
            ActiveDriver::Local => Self::stub_reason(&LOCAL_DRIVER, prompt_vector),
            ActiveDriver::Cloud => Self::stub_reason(&CLOUD_DRIVER, prompt_vector),
            ActiveDriver::Live => match &self.live {
                Some(driver) => match driver.reason(prompt_vector).await {
                    Ok(completion) => UMIResult {
                        output: completion.output,
                        tokens_used: completion.tokens_used,
                        model: completion.model,
                        mode: "SYS_REASON".to_string(),
                        error: None,
                    },
                    Err(err) => {
                        tracing::warn!("live model driver error: {}", err);
                        UMIResult {
                            output: Vec::new(),
                            tokens_used: 0,
                            model: LIVE_DRIVER_NAME.to_string(),
                            mode: "UMI_ERROR".to_string(),
                            error: Some(err),
                        }
                    }
                },
                None => Self::stub_reason(&LOCAL_DRIVER, prompt_vector),
            },
            ActiveDriver::Runtime => {
                let mut drivers = self.runtime_drivers.lock().unwrap();
                let d = drivers.values_mut().next();
                match d {
                    Some(d) => {
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
                            error: None,
                        }
                    }
                    // No runtime driver registered: fall back to the local stub.
                    None => Self::stub_reason(&LOCAL_DRIVER, prompt_vector),
                }
            }
        }
    }

    pub fn embed(&self, data: &[String]) -> Vec<f64> {
        let modulus = match self.active_kind() {
            ActiveDriver::Local => {
                LOCAL_DRIVER.calls.fetch_add(1, Ordering::Relaxed);
                LOCAL_DRIVER.embed_modulus
            }
            ActiveDriver::Cloud => {
                CLOUD_DRIVER.calls.fetch_add(1, Ordering::Relaxed);
                CLOUD_DRIVER.embed_modulus
            }
            // Embeddings still use the deterministic local hash: the live
            // driver covers reasoning only until `/embeddings` is wired.
            ActiveDriver::Live => LIVE_EMBED_MODULUS,
            ActiveDriver::Runtime => {
                let mut drivers = self.runtime_drivers.lock().unwrap();
                match drivers.values_mut().next() {
                    Some(d) => {
                        d.calls += 1;
                        d.embed_modulus
                    }
                    None => LOCAL_DRIVER.embed_modulus,
                }
            }
        };

        hash_embedding(data, modulus)
    }

    pub fn compress(&self, data: &[String]) -> Value {
        let driver_name = self.active_name();
        match self.active_kind() {
            ActiveDriver::Local | ActiveDriver::Cloud => {
                self.with_static_driver(|d| d.calls.fetch_add(1, Ordering::Relaxed));
            }
            ActiveDriver::Runtime => {
                let mut drivers = self.runtime_drivers.lock().unwrap();
                if let Some(d) = drivers.values_mut().next() {
                    d.calls += 1;
                }
            }
            ActiveDriver::Live => {}
        }
        serde_json::json!({
            "original_length": data.len(),
            "driver": driver_name,
            "compressed_at": chrono::Utc::now().to_rfc3339(),
        })
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
        if let Some(live) = &self.live {
            metrics.push(DriverMetrics {
                name: LIVE_DRIVER_NAME.to_string(),
                capability_level: LIVE_DRIVER_CAPABILITY,
                calls: live.calls(),
                total_tokens: live.total_tokens(),
            });
        }
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

fn hash_embedding(data: &[String], modulus: u64) -> Vec<f64> {
    use std::hash::{Hash, Hasher};
    let modulus = modulus.max(1);
    data.iter()
        .map(|s| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            (h.finish() % modulus) as f64 / modulus as f64
        })
        .collect()
}
