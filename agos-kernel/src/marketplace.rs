use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

use crate::kernel::agOSKernel;
use crate::orchestrator::{WorkflowDefinition, WorkflowStep};
use crate::sandbox::AAppLoader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListing {
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub token_quota: u64,
    pub priority: u32,
    pub has_workflow: bool,
    pub states: usize,
    pub tool_count: usize,
    pub source: String,
    pub installed: bool,
}

impl AgentListing {
    pub fn from_aapp_path(path: &Path, installed: bool) -> Option<Self> {
        let pkg = AAppLoader::load_from_path(path).ok()?;
        let description = pkg
            .manifest
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let capabilities: Vec<String> = pkg
            .manifest
            .get("capabilities")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_else(|| {
                pkg.manifest
                    .get("permissions")
                    .and_then(|p| p.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default()
            });
        let token_quota = pkg
            .manifest
            .get("limits")
            .and_then(|l| l.get("max_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        let priority = pkg
            .manifest
            .get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let states = pkg
            .logic_graphs
            .get("states")
            .and_then(|s| s.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        let version = pkg
            .manifest
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0")
            .to_string();
        let permissions: Vec<String> = pkg
            .manifest
            .get("permissions")
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let name = pkg.manifest.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

        Some(AgentListing {
            name,
            version,
            description,
            capabilities,
            permissions,
            token_quota,
            priority,
            has_workflow: pkg.workflow.is_some(),
            states,
            tool_count: pkg.tools.len(),
            source: "local".to_string(),
            installed,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSearchQuery {
    pub query: Option<String>,
    pub capability: Option<String>,
    pub min_priority: Option<u32>,
    pub max_priority: Option<u32>,
    pub has_workflow: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecommendation {
    pub goal: String,
    pub recommended_steps: Vec<RecommendedStep>,
    pub workflow: WorkflowDefinition,
    pub reasoning: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedStep {
    pub step_id: String,
    pub description: String,
    pub recommended_agent: String,
    pub confidence: f64,
    pub alternatives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBuildRequest {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub token_quota: u64,
    pub priority: u32,
    pub states: Vec<BuildState>,
    pub template_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildState {
    pub name: String,
    pub transitions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBuildResult {
    pub name: String,
    pub path: String,
    pub manifest: Value,
    pub logic_graphs: Value,
    pub installed: bool,
}

pub struct AgentMarketplace {
    registry: Mutex<HashMap<String, AgentListing>>,
    templates: Mutex<HashMap<String, AgentTemplate>>,
}

impl Default for AgentMarketplace {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentMarketplace {
    pub fn new() -> Self {
        let marketplace = AgentMarketplace {
            registry: Mutex::new(HashMap::new()),
            templates: Mutex::new(HashMap::new()),
        };
        marketplace.init_templates();
        marketplace
    }

    fn init_templates(&self) {
        let mut templates = self.templates.lock().unwrap();

        templates.insert("data_analyzer".to_string(), AgentTemplate {
            name: "data_analyzer".to_string(),
            description: "Analyzes data and returns structured insights".to_string(),
            default_permissions: vec!["memory:page".to_string(), "umi:reason".to_string()],
            default_token_quota: 5000,
            default_priority: 2,
            default_states: vec![
                ("analyze".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("complete".to_string(), "__terminal__".to_string());
                    m.insert("error".to_string(), "error".to_string());
                    m
                }),
                ("error".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("retry".to_string(), "analyze".to_string());
                    m
                }),
            ],
        });

        templates.insert("report_generator".to_string(), AgentTemplate {
            name: "report_generator".to_string(),
            description: "Generates structured reports from input data".to_string(),
            default_permissions: vec!["memory:page".to_string(), "umi:reason".to_string()],
            default_token_quota: 8000,
            default_priority: 2,
            default_states: vec![
                ("collect".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("data_ready".to_string(), "generate".to_string());
                    m
                }),
                ("generate".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("complete".to_string(), "__terminal__".to_string());
                    m
                }),
            ],
        });

        templates.insert("validator".to_string(), AgentTemplate {
            name: "validator".to_string(),
            description: "Validates data against rules and constraints".to_string(),
            default_permissions: vec!["financial:read".to_string(), "financial:validate".to_string(), "umi:reason".to_string()],
            default_token_quota: 3000,
            default_priority: 1,
            default_states: vec![
                ("validate".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("passed".to_string(), "__terminal__".to_string());
                    m.insert("failed".to_string(), "flag".to_string());
                    m
                }),
                ("flag".to_string(), {
                    let mut m = HashMap::new();
                    m.insert("resolved".to_string(), "__terminal__".to_string());
                    m
                }),
            ],
        });
    }

    pub fn scan_local(&self, aapps_path: &str) -> Vec<AgentListing> {
        let path = Path::new(aapps_path);
        if !path.exists() || !path.is_dir() {
            return Vec::new();
        }

        let mut listings = Vec::new();
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let dir_path = entry.path();
                if dir_path.is_dir() {
                    let manifest_path = dir_path.join("manifest.json");
                    if manifest_path.exists() {
                        if let Some(listing) = AgentListing::from_aapp_path(&dir_path, true) {
                            listings.push(listing);
                        }
                    }
                }
            }
        }

        let mut registry = self.registry.lock().unwrap();
        for listing in &listings {
            registry.insert(listing.name.clone(), listing.clone());
        }

        listings
    }

    pub fn list_agents(&self) -> Vec<AgentListing> {
        let registry = self.registry.lock().unwrap();
        let mut agents: Vec<AgentListing> = registry.values().cloned().collect();
        agents.sort_by(|a, b| a.name.cmp(&b.name));
        agents
    }

    pub fn get_agent(&self, name: &str) -> Option<AgentListing> {
        self.registry.lock().unwrap().get(name).cloned()
    }

    pub fn search_agents(&self, query: &AgentSearchQuery) -> Vec<AgentListing> {
        let registry = self.registry.lock().unwrap();
        let mut results: Vec<AgentListing> = registry.values().cloned().collect();

        if let Some(ref q) = query.query {
            let q_lower = q.to_lowercase();
            results.retain(|a| {
                a.name.to_lowercase().contains(&q_lower)
                    || a.description.to_lowercase().contains(&q_lower)
                    || a.capabilities.iter().any(|c| c.to_lowercase().contains(&q_lower))
            });
        }

        if let Some(ref cap) = query.capability {
            let cap_lower = cap.to_lowercase();
            results.retain(|a| a.capabilities.iter().any(|c| c.to_lowercase().contains(&cap_lower)));
        }

        if let Some(min) = query.min_priority {
            results.retain(|a| a.priority >= min);
        }
        if let Some(max) = query.max_priority {
            results.retain(|a| a.priority <= max);
        }
        if let Some(hw) = query.has_workflow {
            results.retain(|a| a.has_workflow == hw);
        }

        let limit = query.limit.unwrap_or(20);
        results.truncate(limit);
        results
    }

    pub fn list_templates(&self) -> Vec<AgentTemplateInfo> {
        let templates = self.templates.lock().unwrap();
        templates
            .values()
            .map(|t| AgentTemplateInfo {
                name: t.name.clone(),
                description: t.description.clone(),
                default_permissions: t.default_permissions.clone(),
                default_token_quota: t.default_token_quota,
                default_priority: t.default_priority,
            })
            .collect()
    }

    pub fn build_agent(
        &self,
        request: &AgentBuildRequest,
        aapps_path: &str,
        kernel: &agOSKernel,
    ) -> Result<AgentBuildResult, String> {
        let agent_dir = Path::new(aapps_path).join(&request.name);
        std::fs::create_dir_all(&agent_dir)
            .map_err(|e| format!("Failed to create agent directory: {}", e))?;

        let states_obj: HashMap<String, Value> = if request.states.is_empty() {
            let mut default_states = HashMap::new();
            let mut transitions = HashMap::new();
            transitions.insert("complete".to_string(), "__terminal__".to_string());
            default_states.insert(
                "start".to_string(),
                serde_json::json!({"transitions": transitions}),
            );
            default_states
        } else {
            request
                .states
                .iter()
                .map(|s| {
                    (
                        s.name.clone(),
                        serde_json::json!({"transitions": s.transitions}),
                    )
                })
                .collect()
        };

        let manifest = serde_json::json!({
            "name": request.name,
            "version": "1.0.0",
            "description": request.description,
            "capabilities": request.capabilities,
            "permissions": request.permissions,
            "limits": {
                "max_tokens": request.token_quota,
                "max_subagents": 3
            },
            "priority": request.priority,
            "goal_metric": 0.9,
        });

        let logic_graphs = serde_json::json!({
            "initial_state": "start",
            "states": states_obj,
        });

        let manifest_path = agent_dir.join("manifest.json");
        let logic_path = agent_dir.join("logic_graphs.json");

        std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap())
            .map_err(|e| format!("Failed to write manifest: {}", e))?;

        std::fs::write(&logic_path, serde_json::to_string_pretty(&logic_graphs).unwrap())
            .map_err(|e| format!("Failed to write logic_graphs: {}", e))?;

        let tools_dir = agent_dir.join("tools");
        std::fs::create_dir_all(&tools_dir).ok();

        let tests_dir = agent_dir.join("validation_tests");
        std::fs::create_dir_all(&tests_dir).ok();

        self.scan_local(aapps_path);

        kernel.memory.log_episodic(
            "marketplace",
            "AGENT_BUILT",
            serde_json::json!({
                "name": &request.name,
                "description": &request.description,
                "capabilities": &request.capabilities,
            }),
            None,
        );

        Ok(AgentBuildResult {
            name: request.name.clone(),
            path: agent_dir.to_string_lossy().to_string(),
            manifest,
            logic_graphs,
            installed: true,
        })
    }

    pub async fn recommend_agents(
        &self,
        goal: &str,
        kernel: &agOSKernel,
    ) -> Result<AgentRecommendation, String> {
        let available = self.list_agents();
        let agent_list: Vec<String> = available.iter().map(|a| {
            format!("- {}: {} (capabilities: {})", a.name, a.description, a.capabilities.join(", "))
        }).collect();
        let agent_catalog = agent_list.join("\n");

        let prompt = vec![
            format!(
                "You are agOS's agent recommendation engine. Given a user goal and a catalog of available agents, \
                 design an optimal workflow.\n\n\
                 USER GOAL: {goal}\n\n\
                 AVAILABLE AGENTS:\n{agent_catalog}\n\n\
                 INSTRUCTIONS:\n\
                 1. Break the goal into sequential or parallel steps.\n\
                 2. For each step, recommend the best-matching agent from the catalog.\n\
                 3. Provide a confidence score (0.0-1.0) for each recommendation.\n\
                 4. List alternative agents that could also work.\n\
                 5. Explain your reasoning briefly for each step.\n\n\
                 Output format (one step per line, pipe-separated):\n\
                 STEP_ID | DESCRIPTION | RECOMMENDED_AGENT | CONFIDENCE | ALTERNATIVES | REASONING",
                goal = goal,
                agent_catalog = agent_catalog,
            ),
            "Workflow design:".to_string(),
        ];

        let result = kernel.umi.reason(&prompt, &Value::Null).await;

        let mut steps = Vec::new();
        let mut reasoning = Vec::new();
        let mut wf_steps = Vec::new();

        for line in &result.output {
            let parts: Vec<&str> = line.split(" | ").collect();
            if parts.len() >= 4 {
                let step_id = parts[0].trim().to_string();
                let description = parts[1].trim().to_string();
                let agent = parts[2].trim().to_string();
                let confidence: f64 = parts[3].trim().parse().unwrap_or(0.5);
                let alternatives: Vec<String> = parts.get(4)
                    .map(|s| s.split(',').map(|a| a.trim().to_string()).collect())
                    .unwrap_or_default();
                let reason = parts.get(5).map(|s| s.trim().to_string()).unwrap_or_default();

                steps.push(RecommendedStep {
                    step_id: step_id.clone(),
                    description: description.clone(),
                    recommended_agent: agent.clone(),
                    confidence,
                    alternatives,
                });

                wf_steps.push(WorkflowStep {
                    id: step_id.clone(),
                    agent: agent.clone(),
                    timeout_secs: Some(60),
                    on_success: None,
                    on_failure: None,
                    parallel: None,
                    join: None,
                    loop_type: None,
                    loop_condition: None,
                    max_iterations: None,
                });

                if !reason.is_empty() {
                    reasoning.push(format!("Step '{}': {}", step_id, reason));
                }
            }
        }

        if wf_steps.is_empty() {
            return Err("Could not derive workflow steps from goal".to_string());
        }

        for i in 0..wf_steps.len() - 1 {
            wf_steps[i].on_success = Some(wf_steps[i + 1].id.clone());
        }
        if let Some(last) = wf_steps.last_mut() {
            last.on_success = Some("__terminal__".to_string());
        }

        let workflow = WorkflowDefinition {
            workflow: format!("auto-{}", &Uuid::new_v4().to_string()[..8]),
            steps: wf_steps,
        };

        kernel.memory.log_episodic(
            "marketplace",
            "AGENTS_RECOMMENDED",
            serde_json::json!({
                "goal": goal,
                "steps_recommended": steps.len(),
                "workflow_id": &workflow.workflow,
            }),
            None,
        );

        Ok(AgentRecommendation {
            goal: goal.to_string(),
            recommended_steps: steps,
            workflow,
            reasoning,
        })
    }
}

#[allow(dead_code)]
struct AgentTemplate {
    name: String,
    description: String,
    default_permissions: Vec<String>,
    default_token_quota: u64,
    default_priority: u32,
    default_states: Vec<(String, HashMap<String, String>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplateInfo {
    pub name: String,
    pub description: String,
    pub default_permissions: Vec<String>,
    pub default_token_quota: u64,
    pub default_priority: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::umi::UnifiedModelInterface;

    fn test_kernel() -> std::sync::Arc<agOSKernel> {
        std::sync::Arc::new(agOSKernel::new(UnifiedModelInterface::new(), "127.0.0.1:0", None))
    }

    #[test]
    fn test_marketplace_creation() {
        let mp = AgentMarketplace::new();
        let agents = mp.list_agents();
        assert!(agents.is_empty());
    }

    #[test]
    fn test_scan_local_directory() {
        let mp = AgentMarketplace::new();
        let listings = mp.scan_local("aapps");
        assert!(!listings.is_empty(), "should find at least financial_auditor and accountant");
        assert!(listings.iter().any(|a| a.name == "Financial Auditor"));
    }

    #[test]
    fn test_search_by_capability() {
        let mp = AgentMarketplace::new();
        mp.scan_local("aapps");

        let query = AgentSearchQuery {
            query: None,
            capability: Some("financial".to_string()),
            min_priority: None,
            max_priority: None,
            has_workflow: None,
            limit: None,
        };
        let results = mp.search_agents(&query);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_get_agent() {
        let mp = AgentMarketplace::new();
        mp.scan_local("aapps");
        let agent = mp.get_agent("Financial Auditor");
        assert!(agent.is_some());
        assert_eq!(agent.unwrap().token_quota, 5000);
    }

    #[test]
    fn test_templates_exist() {
        let mp = AgentMarketplace::new();
        let templates = mp.list_templates();
        assert!(templates.len() >= 3);
        assert!(templates.iter().any(|t| t.name == "data_analyzer"));
    }

    #[test]
    fn test_build_agent() {
        let mp = AgentMarketplace::new();
        let kernel = test_kernel();

        let request = AgentBuildRequest {
            name: "test-builder-agent".to_string(),
            description: "A test agent".to_string(),
            capabilities: vec!["test:run".to_string()],
            permissions: vec!["umi:reason".to_string()],
            token_quota: 2000,
            priority: 1,
            states: vec![
                BuildState {
                    name: "start".to_string(),
                    transitions: {
                        let mut m = HashMap::new();
                        m.insert("done".to_string(), "__terminal__".to_string());
                        m
                    },
                },
            ],
            template_name: None,
        };

        let result = mp.build_agent(&request, "/tmp/opencode/aapps_test", &kernel);
        assert!(result.is_ok());
        let build = result.unwrap();
        assert_eq!(build.name, "test-builder-agent");

        let _ = std::fs::remove_dir_all("/tmp/opencode/aapps_test/test-builder-agent");
    }

    #[tokio::test]
    async fn test_recommend_agents() {
        let mp = AgentMarketplace::new();
        let kernel = test_kernel();
        mp.scan_local("aapps");

        let result = mp.recommend_agents("Audit the financial records for Q1", &kernel).await;
        assert!(result.is_ok());
        let rec = result.unwrap();
        assert!(!rec.recommended_steps.is_empty());
        assert!(!rec.workflow.steps.is_empty());
    }
}
