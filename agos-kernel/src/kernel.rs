use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthConfig;
use crate::cipc::CIPCBus;
use crate::grid::CognitiveGrid;
use crate::memory::HierarchicalMemoryManager;
use crate::scheduler::{CognitiveProcessContext, TokenScheduler};
use crate::security::SecurityEngine;
use crate::umi::UnifiedModelInterface;

#[cfg(feature = "embodiment")]
use crate::embodiment::EmbodimentLayer;

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
    #[cfg(feature = "embodiment")]
    pub embodiment: EmbodimentLayer,
    panic_log: std::sync::RwLock<Vec<PanicLog>>,
}

impl agOSKernel {
    pub fn new(umi: UnifiedModelInterface, listen_addr: &str) -> Self {
        agOSKernel {
            scheduler: TokenScheduler::new(),
            memory: HierarchicalMemoryManager::new(),
            cipc: CIPCBus::new(),
            security: SecurityEngine::new(),
            umi,
            grid: CognitiveGrid::new(listen_addr),
            auth: std::sync::Mutex::new(AuthConfig::default()),
            #[cfg(feature = "embodiment")]
            embodiment: EmbodimentLayer::new(),
            panic_log: std::sync::RwLock::new(Vec::new()),
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

    pub fn hot_swap_umi(&self, driver_name: &str) -> Result<(), String> {
        self.umi.hot_swap(driver_name)
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
}
