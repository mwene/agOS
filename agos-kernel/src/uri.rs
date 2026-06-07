use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

const TOKEN_COST_SIM_ARM: u64 = 2;
const TOKEN_COST_SIM_SERVO: u64 = 1;

const DRIVER_SIM_ARM: u32 = 0;
const DRIVER_SIM_SERVO: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoboticsResult {
    pub output: Vec<String>,
    pub tokens_used: u64,
    pub driver: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct URIDriverMetrics {
    pub name: String,
    pub capability_level: u32,
    pub calls: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct URIDriverInfo {
    pub name: String,
    pub capability_level: u32,
    pub token_cost: u64,
    pub driver_type: String,
    pub calls: u64,
    pub total_tokens: u64,
    pub hardware_type: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct URIRuntimeDriverState {
    pub name: String,
    pub capability_level: u32,
    pub token_cost: u64,
    pub calls: u64,
    pub total_tokens: u64,
    pub config: HashMap<String, String>,
    pub hardware_type: String,
}

struct RoboticsDriverState {
    name: &'static str,
    capability_level: u32,
    calls: AtomicU64,
    total_tokens: AtomicU64,
    token_cost: u64,
    hardware_type: &'static str,
}

static SIM_ARM_DRIVER: RoboticsDriverState = RoboticsDriverState {
    name: "SimulatedArm-v0",
    capability_level: 3,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: TOKEN_COST_SIM_ARM,
    hardware_type: "robot_arm",
};

static SIM_SERVO_DRIVER: RoboticsDriverState = RoboticsDriverState {
    name: "SimulatedServo-v0",
    capability_level: 1,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: TOKEN_COST_SIM_SERVO,
    hardware_type: "servo",
};

pub struct UnifiedRoboticsInterface {
    active: AtomicU32,
    runtime_drivers: Mutex<HashMap<String, URIRuntimeDriverState>>,
}

impl Default for UnifiedRoboticsInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl UnifiedRoboticsInterface {
    pub fn new() -> Self {
        UnifiedRoboticsInterface {
            active: AtomicU32::new(DRIVER_SIM_ARM),
            runtime_drivers: Mutex::new(HashMap::new()),
        }
    }

    pub fn active_name(&self) -> String {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| d.name.to_string());
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().map(|d| d.name.clone()).unwrap_or_else(|| SIM_ARM_DRIVER.name.to_string())
    }

    pub fn active_capability(&self) -> u32 {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| d.capability_level);
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().map(|d| d.capability_level).unwrap_or(SIM_ARM_DRIVER.capability_level)
    }

    pub fn active_hardware_type(&self) -> String {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| d.hardware_type.to_string());
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().map(|d| d.hardware_type.clone()).unwrap_or_else(|| "unknown".to_string())
    }

    pub fn hot_swap(&self, driver_name: &str) -> Result<(), String> {
        match driver_name {
            "SimulatedArm-v0" => {
                self.active.store(DRIVER_SIM_ARM, Ordering::Relaxed);
                Ok(())
            }
            "SimulatedServo-v0" => {
                self.active.store(DRIVER_SIM_SERVO, Ordering::Relaxed);
                Ok(())
            }
            _ => {
                let drivers = self.runtime_drivers.lock().unwrap();
                if drivers.contains_key(driver_name) {
                    self.active.store(DRIVER_SIM_SERVO + 1, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(format!(
                        "Hardware driver '{}' not found. Available: SimulatedArm-v0, SimulatedServo-v0, and {} runtime driver(s)",
                        driver_name,
                        drivers.len()
                    ))
                }
            }
        }
    }

    pub fn list_drivers(&self) -> Vec<URIDriverInfo> {
        let mut list = Vec::new();
        for d in [&SIM_ARM_DRIVER, &SIM_SERVO_DRIVER].iter() {
            list.push(URIDriverInfo {
                name: d.name.to_string(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                driver_type: "builtin".into(),
                calls: d.calls.load(Ordering::Relaxed),
                total_tokens: d.total_tokens.load(Ordering::Relaxed),
                hardware_type: d.hardware_type.to_string(),
                config: HashMap::new(),
            });
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        for d in drivers.values() {
            list.push(URIDriverInfo {
                name: d.name.clone(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                driver_type: "runtime".into(),
                calls: d.calls,
                total_tokens: d.total_tokens,
                hardware_type: d.hardware_type.clone(),
                config: d.config.clone(),
            });
        }
        list
    }

    pub fn get_driver(&self, name: &str) -> Option<URIDriverInfo> {
        if let Some(d) = [&SIM_ARM_DRIVER, &SIM_SERVO_DRIVER].iter().find(|d| d.name == name) {
            return Some(URIDriverInfo {
                name: d.name.to_string(),
                capability_level: d.capability_level,
                token_cost: d.token_cost,
                driver_type: "builtin".into(),
                calls: d.calls.load(Ordering::Relaxed),
                total_tokens: d.total_tokens.load(Ordering::Relaxed),
                hardware_type: d.hardware_type.to_string(),
                config: HashMap::new(),
            });
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.get(name).map(|d| URIDriverInfo {
            name: d.name.clone(),
            capability_level: d.capability_level,
            token_cost: d.token_cost,
            driver_type: "runtime".into(),
            calls: d.calls,
            total_tokens: d.total_tokens,
            hardware_type: d.hardware_type.clone(),
            config: d.config.clone(),
        })
    }

    pub fn register_driver(
        &self,
        name: &str,
        capability_level: u32,
        token_cost: u64,
        hardware_type: &str,
        config: HashMap<String, String>,
    ) -> Result<(), String> {
        if name.is_empty() {
            return Err("Hardware driver name cannot be empty".into());
        }
        if name == "SimulatedArm-v0" || name == "SimulatedServo-v0" {
            return Err(format!("Cannot register driver with reserved name '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        if drivers.contains_key(name) {
            return Err(format!("Hardware driver '{}' already registered", name));
        }
        drivers.insert(
            name.to_string(),
            URIRuntimeDriverState {
                name: name.to_string(),
                capability_level,
                token_cost,
                calls: 0,
                total_tokens: 0,
                config,
                hardware_type: hardware_type.to_string(),
            },
        );
        Ok(())
    }

    pub fn unregister_driver(&self, name: &str) -> Result<(), String> {
        if name == "SimulatedArm-v0" || name == "SimulatedServo-v0" {
            return Err(format!("Cannot unregister built-in hardware driver '{}'", name));
        }
        let mut drivers = self.runtime_drivers.lock().unwrap();
        drivers.remove(name).ok_or_else(|| format!("Hardware driver '{}' not found", name))?;
        Ok(())
    }

    fn with_static_driver<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&'static RoboticsDriverState) -> R,
    {
        match self.active.load(Ordering::Relaxed) {
            DRIVER_SIM_SERVO => f(&SIM_SERVO_DRIVER),
            _ => f(&SIM_ARM_DRIVER),
        }
    }

    fn resolve_driver(&self) -> Option<URIRuntimeDriverState> {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return None;
        }
        let drivers = self.runtime_drivers.lock().unwrap();
        drivers.values().next().cloned()
    }

    /// SYS_SENSE — read from any hardware sensor
    pub fn sense(&self, sensor_id: &str) -> RoboticsResult {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                d.total_tokens.fetch_add(d.token_cost, Ordering::Relaxed);
                RoboticsResult {
                    output: vec![format!("{}:sense({}) -> OK", d.name, sensor_id)],
                    tokens_used: d.token_cost,
                    driver: d.name.to_string(),
                    mode: "SYS_SENSE".to_string(),
                }
            });
        }
        if let Some(d) = self.resolve_driver() {
            let tokens = d.token_cost;
            RoboticsResult {
                output: vec![format!("{}:sense({}) -> OK", d.name, sensor_id)],
                tokens_used: tokens,
                driver: d.name.clone(),
                mode: "SYS_SENSE".to_string(),
            }
        } else {
            let d = &SIM_ARM_DRIVER;
            d.calls.fetch_add(1, Ordering::Relaxed);
            d.total_tokens.fetch_add(d.token_cost, Ordering::Relaxed);
            RoboticsResult {
                output: vec![format!("{}:sense({}) -> OK", d.name, sensor_id)],
                tokens_used: d.token_cost,
                driver: d.name.to_string(),
                mode: "SYS_SENSE".to_string(),
            }
        }
    }

    /// SYS_ACTUATE — command any actuator (motor, servo, gripper, etc.)
    pub fn actuate(&self, actuator_id: &str, command: &Value) -> RoboticsResult {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                let tokens = d.token_cost * 2;
                d.total_tokens.fetch_add(tokens, Ordering::Relaxed);
                RoboticsResult {
                    output: vec![format!("{}:actuate({}, {}) -> OK", d.name, actuator_id, command)],
                    tokens_used: tokens,
                    driver: d.name.to_string(),
                    mode: "SYS_ACTUATE".to_string(),
                }
            });
        }
        if let Some(d) = self.resolve_driver() {
            let tokens = d.token_cost * 2;
            RoboticsResult {
                output: vec![format!("{}:actuate({}, {}) -> OK", d.name, actuator_id, command)],
                tokens_used: tokens,
                driver: d.name.clone(),
                mode: "SYS_ACTUATE".to_string(),
            }
        } else {
            let d = &SIM_ARM_DRIVER;
            d.calls.fetch_add(1, Ordering::Relaxed);
            d.total_tokens.fetch_add(d.token_cost * 2, Ordering::Relaxed);
            RoboticsResult {
                output: vec![format!("{}:actuate({}, {}) -> OK", d.name, actuator_id, command)],
                tokens_used: d.token_cost * 2,
                driver: d.name.to_string(),
                mode: "SYS_ACTUATE".to_string(),
            }
        }
    }

    /// SYS_TRANSFORM — coordinate transforms and kinematics (forward/inverse)
    pub fn transform(&self, command: &str, args: &Value) -> RoboticsResult {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                let tokens = d.token_cost * 3;
                d.total_tokens.fetch_add(tokens, Ordering::Relaxed);
                RoboticsResult {
                    output: vec![format!("{}:transform({}, {}) -> OK", d.name, command, args)],
                    tokens_used: tokens,
                    driver: d.name.to_string(),
                    mode: "SYS_TRANSFORM".to_string(),
                }
            });
        }
        if let Some(d) = self.resolve_driver() {
            let tokens = d.token_cost * 3;
            RoboticsResult {
                output: vec![format!("{}:transform({}, {}) -> OK", d.name, command, args)],
                tokens_used: tokens,
                driver: d.name.clone(),
                mode: "SYS_TRANSFORM".to_string(),
            }
        } else {
            let d = &SIM_ARM_DRIVER;
            d.calls.fetch_add(1, Ordering::Relaxed);
            d.total_tokens.fetch_add(d.token_cost * 3, Ordering::Relaxed);
            RoboticsResult {
                output: vec![format!("{}:transform({}, {}) -> OK", d.name, command, args)],
                tokens_used: d.token_cost * 3,
                driver: d.name.to_string(),
                mode: "SYS_TRANSFORM".to_string(),
            }
        }
    }

    /// SYS_STATUS — device health, calibration state, fault flags
    pub fn status(&self, device_id: &str) -> RoboticsResult {
        let active_idx = self.active.load(Ordering::Relaxed);
        if active_idx <= DRIVER_SIM_SERVO {
            return self.with_static_driver(|d| {
                d.calls.fetch_add(1, Ordering::Relaxed);
                d.total_tokens.fetch_add(d.token_cost, Ordering::Relaxed);
                RoboticsResult {
                    output: vec![format!("{}:status({}) -> online, calibrated", d.name, device_id)],
                    tokens_used: d.token_cost,
                    driver: d.name.to_string(),
                    mode: "SYS_STATUS".to_string(),
                }
            });
        }
        if let Some(d) = self.resolve_driver() {
            let tokens = d.token_cost;
            RoboticsResult {
                output: vec![format!("{}:status({}) -> online, calibrated", d.name, device_id)],
                tokens_used: tokens,
                driver: d.name.clone(),
                mode: "SYS_STATUS".to_string(),
            }
        } else {
            let d = &SIM_ARM_DRIVER;
            d.calls.fetch_add(1, Ordering::Relaxed);
            d.total_tokens.fetch_add(d.token_cost, Ordering::Relaxed);
            RoboticsResult {
                output: vec![format!("{}:status({}) -> online, calibrated", d.name, device_id)],
                tokens_used: d.token_cost,
                driver: d.name.to_string(),
                mode: "SYS_STATUS".to_string(),
            }
        }
    }

    pub fn get_metrics(&self) -> Vec<URIDriverMetrics> {
        let mut metrics = Vec::new();
        metrics.push(URIDriverMetrics {
            name: SIM_ARM_DRIVER.name.to_string(),
            capability_level: SIM_ARM_DRIVER.capability_level,
            calls: SIM_ARM_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: SIM_ARM_DRIVER.total_tokens.load(Ordering::Relaxed),
        });
        metrics.push(URIDriverMetrics {
            name: SIM_SERVO_DRIVER.name.to_string(),
            capability_level: SIM_SERVO_DRIVER.capability_level,
            calls: SIM_SERVO_DRIVER.calls.load(Ordering::Relaxed),
            total_tokens: SIM_SERVO_DRIVER.total_tokens.load(Ordering::Relaxed),
        });
        let drivers = self.runtime_drivers.lock().unwrap();
        for d in drivers.values() {
            metrics.push(URIDriverMetrics {
                name: d.name.clone(),
                capability_level: d.capability_level,
                calls: d.calls,
                total_tokens: d.total_tokens,
            });
        }
        metrics
    }
}
