use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::kernel::agOSKernel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopType {
    RepeatUntil,
    While,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub agent: String,
    pub timeout_secs: Option<u64>,
    pub on_success: Option<String>,
    pub on_failure: Option<String>,
    pub parallel: Option<Vec<WorkflowStep>>,
    pub join: Option<String>,
    pub loop_type: Option<LoopType>,
    pub loop_condition: Option<String>,
    pub max_iterations: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub workflow: String,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: String,
    pub name: String,
    pub status: WorkflowStatus,
    pub current_step: Option<String>,
    pub step_results: HashMap<String, WorkflowStepResult>,
    pub created_at: String,
    pub updated_at: String,
    pub definition: Option<WorkflowDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Running,
    Completed,
    Failed(String),
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepResult {
    pub step_id: String,
    pub agent: String,
    pub status: String,
    pub pid: Option<String>,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

pub struct SubAgentOrchestrator {
    workflows: RwLock<Vec<WorkflowInstance>>,
}

impl Default for SubAgentOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl SubAgentOrchestrator {
    pub fn new() -> Self {
        SubAgentOrchestrator {
            workflows: RwLock::new(Vec::new()),
        }
    }

    pub async fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> WorkflowInstance {
        let instance = WorkflowInstance {
            id: format!("wf-{}", &Uuid::new_v4().to_string()[..12]),
            name: definition.workflow.clone(),
            status: WorkflowStatus::Running,
            current_step: definition.steps.first().map(|s| s.id.clone()),
            step_results: HashMap::new(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            definition: Some(definition),
        };

        let mut workflows = self.workflows.write().await;
        workflows.push(instance.clone());
        instance
    }

    pub async fn submit_workflow_definition(
        &self,
        definition: &WorkflowDefinition,
    ) -> WorkflowInstance {
        let instance = WorkflowInstance {
            id: format!("wf-{}", &Uuid::new_v4().to_string()[..12]),
            name: definition.workflow.clone(),
            status: WorkflowStatus::Running,
            current_step: definition.steps.first().map(|s| s.id.clone()),
            step_results: HashMap::new(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            definition: Some(definition.clone()),
        };

        let mut workflows = self.workflows.write().await;
        workflows.push(instance.clone());
        instance
    }

    pub async fn execute_workflow(
        &self,
        workflow_id: &str,
        kernel: &agOSKernel,
    ) -> Result<(), String> {
        let definition = {
            let workflows = self.workflows.read().await;
            let inst = workflows
                .iter()
                .find(|w| w.id == workflow_id)
                .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?
                .clone();
            inst
        };

        let aapp_base = std::path::Path::new("aapps");
        let definition_data = {
            let path = aapp_base.join(&definition.name);
            let logic_path = path.join("logic_graphs.json");
            if logic_path.exists() {
                let content = std::fs::read_to_string(&logic_path)
                    .map_err(|e| format!("Read workflow DSL: {}", e))?;
                let wf: Value = serde_json::from_str(&content)
                    .map_err(|e| format!("Parse workflow DSL: {}", e))?;
                let steps_val = wf.get("steps")
                    .ok_or_else(|| "No steps in workflow DSL".to_string())?
                    .clone();
                let steps: Vec<WorkflowStep> = serde_json::from_value(steps_val)
                    .map_err(|e| format!("Deserialize steps: {}", e))?;
                WorkflowDefinition {
                    workflow: definition.name.clone(),
                    steps,
                }
            } else if let Some(ref def) = definition.definition {
                def.clone()
            } else {
                return Err(format!("No workflow DSL found for '{}' and no stored definition", definition.name));
            }
        };

        self.execute_steps(&definition.name, &definition_data.steps, None, kernel)
            .await
    }

    async fn execute_steps(
        &self,
        _workflow_name: &str,
        steps: &[WorkflowStep],
        _parent_pid: Option<String>,
        kernel: &agOSKernel,
    ) -> Result<(), String> {
        for step in steps {
            if step.loop_type.is_some() {
                self.execute_loop_step(_workflow_name, step, kernel).await?;
            } else if let Some(ref parallel_steps) = step.parallel {
                let join_type = step.join.as_deref().unwrap_or("all");
                let mut handles = Vec::new();
                for pstep in parallel_steps {
                    let manifest = serde_json::json!({
                        "name": format!("{}:{}", _workflow_name, pstep.agent),
                        "agent": pstep.agent,
                        "limits": { "max_tokens": 5000 },
                        "priority": 2,
                    });
                    handles.push(kernel.spawn_process(manifest, None).await);
                }
                match join_type {
                    "all" => {
                        for handle in handles {
                            let _ = kernel.scheduler.get_process(&handle.pid).await;
                        }
                    }
                    "any" => {
                        if let Some(handle) = handles.first() {
                            let _ = kernel.scheduler.get_process(&handle.pid).await;
                        }
                    }
                    _ => {}
                }
            } else {
                let manifest = serde_json::json!({
                    "name": format!("{}:{}", _workflow_name, step.agent),
                    "agent": step.agent,
                    "limits": { "max_tokens": 5000 },
                    "priority": 2,
                });
                let _ctx = kernel.spawn_process(manifest, None).await;
            }
        }
        Ok(())
    }

    async fn execute_loop_step(
        &self,
        _workflow_name: &str,
        step: &WorkflowStep,
        kernel: &agOSKernel,
    ) -> Result<(), String> {
        let max_iter = step.max_iterations.unwrap_or(10) as usize;
        let loop_type = step.loop_type.as_ref().unwrap();

        for iteration in 0..max_iter {
            let manifest = serde_json::json!({
                "name": format!("{}:{}:iter{}", _workflow_name, step.agent, iteration),
                "agent": step.agent,
                "limits": { "max_tokens": 5000 },
                "priority": 2,
            });
            let _ctx = kernel.spawn_process(manifest, None).await;

            if let Some(ref condition) = step.loop_condition {
                let should_continue = match loop_type {
                    LoopType::RepeatUntil => {
                        let done = self.check_condition(condition, kernel).await;
                        !done
                    }
                    LoopType::While => {
                        self.check_condition(condition, kernel).await
                    }
                };
                if !should_continue {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn check_condition(&self, condition: &str, kernel: &agOSKernel) -> bool {
        let result = kernel.umi.reason(
            &[format!("Evaluate whether this condition is true or false. Respond with only 'true' or 'false'. Condition: {}", condition)],
            &serde_json::json!({"task": "condition_check"}),
        ).await;
        result.output.first().map(|s| s.trim().eq_ignore_ascii_case("true")).unwrap_or(false)
    }

    pub async fn list_workflows(&self) -> Vec<WorkflowInstance> {
        self.workflows.read().await.clone()
    }

    pub async fn get_workflow(&self, id: &str) -> Option<WorkflowInstance> {
        let workflows = self.workflows.read().await;
        workflows.iter().find(|w| w.id == id).cloned()
    }
}

pub fn parse_dsl_from_value(value: &Value) -> Option<WorkflowDefinition> {
    let workflow = value.get("workflow")?.as_str()?.to_string();
    let steps_val = value.get("steps")?;
    let steps: Vec<WorkflowStep> = serde_json::from_value(steps_val.clone()).ok()?;
    Some(WorkflowDefinition { workflow, steps })
}
