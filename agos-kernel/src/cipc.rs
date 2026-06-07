use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub payload: Value,
    pub timestamp: String,
}

pub struct SchemaValidator;

impl SchemaValidator {
    pub fn validate(payload: &Value, schema: &Value) -> bool {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required {
                if let Some(field_name) = field.as_str() {
                    if payload.get(field_name).is_none() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

pub struct CIPCBus {
    messages: std::sync::RwLock<Vec<Message>>,
    route_permissions: std::sync::RwLock<std::collections::HashMap<String, Vec<String>>>,
}

impl Default for CIPCBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CIPCBus {
    pub fn new() -> Self {
        CIPCBus {
            messages: std::sync::RwLock::new(Vec::new()),
            route_permissions: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub fn register_process(&self, pid: &str, allowed_routes: Vec<String>) {
        let mut perms = self.route_permissions.write().unwrap();
        perms.insert(pid.to_string(), allowed_routes);
    }

    pub fn check_route_permission(&self, sender: &str, target: &str) -> bool {
        let perms = self.route_permissions.read().unwrap();
        if let Some(allowed) = perms.get(sender) {
            if allowed.is_empty() {
                return true;
            }
            return allowed.iter().any(|a| a == target);
        }
        true
    }

    pub fn route(&self, from: &str, to: &str, payload: Value) -> Result<Message, String> {
        if !self.check_route_permission(from, to) {
            return Err(format!("Unauthorized cIPC route: {} → {}", from, to));
        }
        let msg = Message {
            id: Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: to.to_string(),
            payload,
            timestamp: Utc::now().to_rfc3339(),
        };
        let mut msgs = self.messages.write().unwrap();
        msgs.push(msg.clone());
        Ok(msg)
    }

    pub fn get_history(&self, pid: Option<&str>, limit: usize) -> Vec<Message> {
        let msgs = self.messages.read().unwrap();
        let iter: Vec<Message> = match pid {
            Some(pid) => msgs
                .iter()
                .filter(|m| m.from == pid || m.to == pid)
                .cloned()
                .collect(),
            None => msgs.clone(),
        };
        iter.into_iter().rev().take(limit).rev().collect()
    }
}
