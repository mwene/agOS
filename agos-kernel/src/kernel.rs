use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::audit_export::{AuditExporter, ExportFormat, ExportResult};
use crate::auth::AuthConfig;
use crate::cipc::CIPCBus;
use crate::governance::{ComplianceReport, ComplianceSummary, ContractEngine, GovernanceContract, GovernancePolicy};
use crate::grid::CognitiveGrid;
use crate::marketplace::{AgentBuildRequest, AgentBuildResult, AgentListing, AgentMarketplace, AgentRecommendation, AgentSearchQuery};
use crate::memory::HierarchicalMemoryManager;
use crate::models::ModelRegistry;
use crate::orchestrator::{SubAgentOrchestrator, WorkflowDefinition};
use crate::scheduler::{CognitiveProcessContext, TokenScheduler};
use crate::security::SecurityEngine;
use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::storage::{PersistentStore, SharedStore};
use std::sync::Arc;
use crate::umi::UnifiedModelInterface;

#[cfg(feature = "embodiment")]
use crate::embodiment::EmbodimentLayer;

#[cfg(feature = "uri")]
use crate::controller::ControllerManager;
#[cfg(feature = "uri")]
use crate::uri::UnifiedRoboticsInterface;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanicLog {
    pub pid: String,
    pub reason: String,
    pub timestamp: String,
    pub tokens_used: u64,
}

#[allow(non_camel_case_types)]
pub struct agOSKernel {
    pub scheduler: TokenScheduler,
    pub memory: HierarchicalMemoryManager,
    pub cipc: CIPCBus,
    pub security: SecurityEngine,
    pub umi: UnifiedModelInterface,
    pub grid: CognitiveGrid,
    pub auth: std::sync::Mutex<AuthConfig>,
    pub store: SharedStore,
    pub rate_limiter: RateLimiter,
    pub model_registry: Arc<ModelRegistry>,
    pub orchestrator: SubAgentOrchestrator,
    pub marketplace: AgentMarketplace,
    pub governance: ContractEngine,
    #[cfg(feature = "embodiment")]
    pub embodiment: EmbodimentLayer,
    #[cfg(feature = "uri")]
    pub uri: UnifiedRoboticsInterface,
    #[cfg(feature = "uri")]
    pub controller: ControllerManager,
    panic_log: std::sync::RwLock<Vec<PanicLog>>,
}

impl agOSKernel {
    pub fn new(umi: UnifiedModelInterface, listen_addr: &str, db_path: Option<String>) -> Self {
        let models_path = std::env::var("AGOS_MODELS_PATH").ok();
        let model_registry = Arc::new(
            models_path
                .as_ref()
                .and_then(|p| ModelRegistry::from_json_file(p).ok())
                .unwrap_or_else(ModelRegistry::builtin),
        );

        let store: SharedStore = db_path
            .map(|p| PersistentStore::open(&p).map(Arc::new))
            .transpose()
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to open persistent store: {}", e);
                None
            });

        let store_for_memory = store.clone();
        let store_for_security = store.clone();

        let mut memory = HierarchicalMemoryManager::new();
        let mut security = SecurityEngine::new();
        memory.set_store(store_for_memory);
        security.set_store(store_for_security);

        let mut scheduler = TokenScheduler::new();
        // share the same registry so scheduler model lookups
        // always see the (possibly user-overridden) pricing data
        scheduler.model_registry = model_registry.clone();

        let marketplace = AgentMarketplace::new();
        marketplace.scan_local("aapps");

        agOSKernel {
            scheduler,
            memory,
            cipc: CIPCBus::new(),
            security,
            umi,
            grid: CognitiveGrid::new(listen_addr),
            auth: std::sync::Mutex::new(AuthConfig::default()),
            rate_limiter: RateLimiter::new(RateLimitConfig::default()),
            model_registry,
            store,
            orchestrator: SubAgentOrchestrator::new(),
            marketplace,
            governance: ContractEngine::new(),
            #[cfg(feature = "embodiment")]
            embodiment: EmbodimentLayer::new(),
            #[cfg(feature = "uri")]
            uri: UnifiedRoboticsInterface::new(),
            #[cfg(feature = "uri")]
            controller: ControllerManager::new(),
            panic_log: std::sync::RwLock::new(Vec::new()),
        }
    }

    fn persist_panic(&self, entry: &PanicLog) {
        if let Some(ref store) = self.store {
            if let Err(e) = store.append_panic(entry) {
                tracing::warn!("Failed to persist panic log: {}", e);
            }
        }
    }

    pub async fn spawn_process(
        &self,
        manifest: Value,
        parent_pid: Option<String>,
    ) -> CognitiveProcessContext {
        let pid = format!("proc-{}", &Uuid::new_v4().to_string()[..12]);
        let mut ctx = CognitiveProcessContext::new(pid.clone(), manifest);
        ctx.parent_pid = parent_pid;
        self.scheduler.register(ctx.clone()).await;

        self.memory.log_episodic(
            &pid,
            "PROCESS_SPAWNED",
            serde_json::json!({
                "pid": &pid,
                "priority": ctx.priority,
                "token_quota": ctx.token_quota,
            }),
            None,
        );
        ctx
    }

    pub async fn kill_process(&self, pid: &str) {
        if let Some(ctx) = self.scheduler.get_process(pid).await {
            self.memory.log_episodic(
                pid,
                "PROCESS_TERMINATED",
                ctx.save_state(),
                None,
            );
            self.memory.flush_l1(pid);
        }
        self.scheduler.unregister(pid).await;
    }

    pub async fn kernel_panic(&self, pid: &str, reason: &str) -> PanicLog {
        let tokens_used = self
            .scheduler
            .get_process(pid)
            .await
            .map(|c| c.tokens_used)
            .unwrap_or(0);

        let entry = PanicLog {
            pid: pid.to_string(),
            reason: reason.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            tokens_used,
        };

        self.panic_log.write().unwrap().push(entry.clone());
        self.persist_panic(&entry);

        self.memory.log_episodic(
            pid,
            "KERNEL_PANIC",
            serde_json::json!({ "reason": reason, "tokens_used": tokens_used }),
            None,
        );

        self.memory.flush_l1(pid);
        self.scheduler.unregister(pid).await;
        self.broadcast_alert(pid, reason);
        entry
    }

    pub fn broadcast_alert(&self, source_pid: &str, message: &str) {
        self.memory.log_episodic(
            "system",
            "ALERT_BROADCAST",
            serde_json::json!({
                "type": "system.alert",
                "source": source_pid,
                "message": message,
                "severity": "CRITICAL",
            }),
            None,
        );
    }

    pub fn get_panic_log(&self) -> Vec<PanicLog> {
        self.panic_log.read().unwrap().clone()
    }

    pub fn list_models(&self) -> Vec<crate::models::TokenModel> {
        self.model_registry.list().into_iter().cloned().collect()
    }

    pub async fn scheduler_policy_name(&self) -> &'static str {
        self.scheduler.policy_name().await
    }

    pub async fn set_scheduler_policy(&self, name: &str) -> Result<(), String> {
        let policy: Box<dyn crate::scheduler::SchedulePolicy> = match name {
            "priority_preemption" => Box::new(crate::scheduler::PriorityPreemption),
            "round_robin" => Box::new(crate::scheduler::RoundRobin),
            "fifo" => Box::new(crate::scheduler::FIFO),
            "token_aware" => Box::new(crate::scheduler::TokenAware),
            _ => return Err(format!("Unknown scheduler policy: {}", name)),
        };
        self.scheduler.set_policy(policy).await;
        Ok(())
    }

    pub fn hot_swap_umi(&self, driver_name: &str) -> Result<(), String> {
        self.umi.hot_swap(driver_name)
    }

    #[cfg(feature = "uri")]
    pub fn hot_swap_uri(&self, driver_name: &str) -> Result<(), String> {
        self.uri.hot_swap(driver_name)
    }

    pub async fn migrate_process(
        &self,
        pid: &str,
        target_node: &str,
    ) -> Result<crate::grid::MigratingProcess, String> {
        let ctx = self
            .scheduler
            .get_process(pid)
            .await
            .ok_or_else(|| format!("Process '{}' not found", pid))?;
        let frozen = self.grid.freeze_process(&ctx);
        self.kill_process(pid).await;
        self.memory.log_episodic(
            pid,
            "PROCESS_MIGRATED_OUT",
            serde_json::json!({
                "pid": pid,
                "target_node": target_node,
                "source_node": self.grid.node_id(),
            }),
            None,
        );
        Ok(frozen)
    }

    pub async fn receive_process(
        &self,
        incoming: crate::grid::MigratingProcess,
    ) -> CognitiveProcessContext {
        let mut ctx = CognitiveProcessContext::new(incoming.pid.clone(), incoming.manifest);
        ctx.token_quota = incoming.token_quota;
        ctx.tokens_used = incoming.tokens_used;
        ctx.priority = incoming.priority;
        ctx.active_l1_context = incoming.active_l1_context;
        ctx.state_machine = incoming.state_machine;
        ctx.parent_pid = incoming.parent_pid;
        ctx.goal_metric = incoming.goal_metric;
        ctx.created_at = incoming.created_at;

        self.scheduler.register(ctx.clone()).await;

        self.memory.log_episodic(
            &incoming.pid,
            "PROCESS_MIGRATED_IN",
            serde_json::json!({
                "pid": &incoming.pid,
                "source_node": &incoming.source_node,
                "local_node": self.grid.node_id(),
            }),
            None,
        );

        ctx
    }

    pub async fn export_audit_log(
        &self,
        format: &str,
        sink: &str,
        since: Option<&str>,
        exporter: Box<dyn AuditExporter>,
    ) -> Result<ExportResult, String> {
        ExportFormat::from_str(format)?;
        let mut entries = self.security.get_audit_log(usize::MAX);
        if let Some(since_str) = since {
            entries.retain(|e| e.timestamp.as_str() >= since_str);
        }
        let count = entries.len();
        let result = ExportResult {
            format: format.to_string(),
            entries_exported: count,
            sink: sink.to_string(),
            timestamp: Utc::now().to_rfc3339(),
        };
        exporter.export(&entries).await?;
        Ok(result)
    }

    pub async fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<crate::orchestrator::WorkflowInstance, String> {
        let instance = self.orchestrator.submit_workflow(definition).await;
        Ok(instance)
    }

    pub async fn execute_workflow(&self, workflow_id: &str) -> Result<(), String> {
        self.orchestrator.execute_workflow(workflow_id, self).await
    }

    pub async fn list_workflows(&self) -> Vec<crate::orchestrator::WorkflowInstance> {
        self.orchestrator.list_workflows().await
    }

    pub async fn get_workflow(&self, id: &str) -> Option<crate::orchestrator::WorkflowInstance> {
        self.orchestrator.get_workflow(id).await
    }

    // ---- Marketplace ----

    pub fn list_marketplace_agents(&self) -> Vec<AgentListing> {
        self.marketplace.list_agents()
    }

    pub fn search_marketplace(&self, query: &AgentSearchQuery) -> Vec<AgentListing> {
        self.marketplace.search_agents(query)
    }

    pub fn get_marketplace_agent(&self, name: &str) -> Option<AgentListing> {
        self.marketplace.get_agent(name)
    }

    pub fn scan_marketplace(&self) -> Vec<AgentListing> {
        self.marketplace.scan_local("aapps")
    }

    pub fn list_templates(&self) -> Vec<crate::marketplace::AgentTemplateInfo> {
        self.marketplace.list_templates()
    }

    pub fn build_agent(&self, request: &AgentBuildRequest) -> Result<AgentBuildResult, String> {
        self.marketplace.build_agent(request, "aapps", self)
    }

    pub async fn recommend_agents_for_goal(&self, goal: &str) -> Result<AgentRecommendation, String> {
        self.marketplace.recommend_agents(goal, self).await
    }

    // ---- Governance ----

    pub fn register_contract(&self, contract: GovernanceContract) -> Result<(), String> {
        self.governance.register_contract(contract)
    }

    pub fn get_contract(&self, contract_id: &str) -> Option<GovernanceContract> {
        self.governance.get_contract(contract_id)
    }

    pub fn list_contracts(&self) -> Vec<GovernanceContract> {
        self.governance.list_contracts()
    }

    pub fn remove_contract(&self, contract_id: &str) -> bool {
        self.governance.remove_contract(contract_id)
    }

    pub fn get_compliance_summary(&self) -> ComplianceSummary {
        self.governance.get_compliance_summary()
    }

    pub fn get_audit_log(&self, limit: Option<usize>) -> Vec<ComplianceReport> {
        self.governance.get_audit_log(limit)
    }

    pub fn create_policy(&self, name: &str, description: &str, contract_ids: Vec<String>) -> GovernancePolicy {
        self.governance.create_policy(name, description, contract_ids)
    }

    pub fn list_policies(&self) -> Vec<GovernancePolicy> {
        self.governance.list_policies()
    }
}
