use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AAppPackage {
    pub name: String,
    pub manifest: Value,
    pub logic_graphs: Value,
    pub tools: HashMap<String, Value>,
    pub validation_tests: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub status: String,
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
    pub token_quota: u64,
    pub states: usize,
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

        Ok(AAppPackage {
            name,
            manifest,
            logic_graphs,
            tools,
            validation_tests,
        })
    }
}
