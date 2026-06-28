use base64::Engine;
use ed25519_dalek::Verifier;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::orchestrator::WorkflowDefinition;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AAppPackage {
    pub name: String,
    pub manifest: Value,
    pub logic_graphs: Value,
    pub tools: HashMap<String, Value>,
    pub validation_tests: Vec<Value>,
    pub workflow: Option<WorkflowDefinition>,
    pub signature_valid: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub status: String,
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
    pub token_quota: u64,
    pub states: usize,
    pub signature_valid: Option<bool>,
    pub has_workflow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifestError {
    pub msg: String,
}

pub struct AAppLoader;

impl AAppLoader {
    pub fn load_from_path(path: &std::path::Path) -> Result<AAppPackage, String> {
        let manifest_path = path.join("manifest.json");
        let logic_path = path.join("logic_graphs.json");
        let tools_dir = path.join("tools");
        let tests_dir = path.join("validation_tests");

        let manifest_content =
            std::fs::read_to_string(&manifest_path).map_err(|e| format!("Missing manifest.json: {}", e))?;
        let manifest: Value =
            serde_json::from_str(&manifest_content).map_err(|e| format!("Invalid manifest.json: {}", e))?;

        let logic_graphs: Value = if logic_path.exists() {
            let content = std::fs::read_to_string(&logic_path).map_err(|e| format!("Read logic_graphs.json: {}", e))?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Value::Null
        };

        let mut tools = HashMap::new();
        if tools_dir.exists() {
            for entry in std::fs::read_dir(&tools_dir).map_err(|e| format!("Read tools dir: {}", e))? {
                let entry = entry.map_err(|e| format!("Tools entry: {}", e))?;
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.ends_with(".json") {
                    let content = std::fs::read_to_string(entry.path())
                        .map_err(|e| format!("Read tool {}: {}", fname, e))?;
                    let tool_val: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Parse tool {}: {}", fname, e))?;
                    tools.insert(fname.replace(".json", ""), tool_val);
                }
            }
        }

        let mut validation_tests = Vec::new();
        if tests_dir.exists() {
            let mut entries: Vec<_> = std::fs::read_dir(&tests_dir)
                .map_err(|e| format!("Read tests dir: {}", e))?
                .filter_map(|e| e.ok())
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.ends_with(".json") {
                    let content = std::fs::read_to_string(entry.path())
                        .map_err(|e| format!("Read test {}: {}", fname, e))?;
                    let test_val: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Parse test {}: {}", fname, e))?;
                    validation_tests.push(test_val);
                }
            }
        }

        let name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let workflow = if logic_path.exists() {
            std::fs::read_to_string(&logic_path)
                .ok()
                .and_then(|content| serde_json::from_str::<Value>(&content).ok())
                .and_then(|val| crate::orchestrator::parse_dsl_from_value(&val))
        } else {
            None
        };

        let signature_valid = verify_aapp_signature(path);

        Ok(AAppPackage {
            name,
            manifest,
            logic_graphs,
            tools,
            validation_tests,
            workflow,
            signature_valid,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AAppInstance {
    pub name: String,
    pub path: String,
    pub manifest: Value,
    pub logic_graphs: Value,
    pub tools: HashMap<String, Value>,
    pub workflow: Option<WorkflowDefinition>,
    pub process: Option<AAppProcessHandle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AAppProcessHandle {
    pub pid: String,
    pub status: String,
}

pub struct AAppBuilder;

impl AAppBuilder {
    pub fn from_template(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        permissions: Vec<String>,
        token_quota: u64,
        template_path: &std::path::Path,
    ) -> Result<Value, String> {
        let pkg = AAppLoader::load_from_path(template_path)?;

        let mut manifest = pkg.manifest.clone();
        if let Some(obj) = manifest.as_object_mut() {
            obj.insert("name".to_string(), Value::String(name.to_string()));
            obj.insert("description".to_string(), Value::String(description.to_string()));
            if !capabilities.is_empty() {
                obj.insert(
                    "capabilities".to_string(),
                    Value::Array(capabilities.iter().map(|c| Value::String(c.clone())).collect()),
                );
            }
            if !permissions.is_empty() {
                obj.insert(
                    "permissions".to_string(),
                    Value::Array(permissions.iter().map(|p| Value::String(p.clone())).collect()),
                );
            }
            if let Some(limits) = obj.get_mut("limits") {
                if let Some(limits_obj) = limits.as_object_mut() {
                    limits_obj.insert(
                        "max_tokens".to_string(),
                        Value::Number(serde_json::Number::from(token_quota)),
                    );
                }
            }
        }
        Ok(manifest)
    }

    pub fn create_standalone(
        name: &str,
        description: &str,
        capabilities: Vec<String>,
        permissions: Vec<String>,
        token_quota: u64,
        states: Option<Value>,
        tools: Option<HashMap<String, Value>>,
    ) -> (Value, Value, HashMap<String, Value>) {
        let manifest = serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "description": description,
            "capabilities": capabilities,
            "permissions": permissions,
            "limits": {
                "max_tokens": token_quota,
                "max_subagents": 3
            },
            "priority": 1,
            "goal_metric": 0.9,
        });

        let logic_graphs = states.unwrap_or_else(|| {
            let mut transitions = std::collections::HashMap::new();
            transitions.insert("complete".to_string(), "__terminal__".to_string());
            serde_json::json!({
                "initial_state": "start",
                "states": {
                    "start": {
                        "transitions": transitions
                    }
                }
            })
        });

        let tools = tools.unwrap_or_default();

        (manifest, logic_graphs, tools)
    }
}

fn verify_aapp_signature(path: &std::path::Path) -> Option<bool> {
    let sig_path = path.join("signature.sig");
    if !sig_path.exists() {
        return None;
    }

    let trusted_key = std::env::var("AGOS_TRUSTED_KEY").ok()?;
    let sig_data = std::fs::read_to_string(&sig_path).ok()?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_data.trim())
        .ok()?;
    let sig_array: [u8; 64] = sig_bytes.try_into().ok()?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_array);

    let pub_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(trusted_key.trim())
        .ok()?;
    let pub_key_array: [u8; 32] = pub_key_bytes.try_into().ok()?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pub_key_array).ok()?;

    let hash = hash_aapp_dir(path);

    Some(verifying_key.verify(&hash, &signature).is_ok())
}

fn hash_aapp_dir(aapp_dir: &std::path::Path) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut entries: Vec<_> = std::fs::read_dir(aapp_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            name_str != "signature.sig" && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        if let Ok(data) = std::fs::read(entry.path()) {
            hasher.update(&data);
        }
    }
    hasher.finalize().to_vec()
}
