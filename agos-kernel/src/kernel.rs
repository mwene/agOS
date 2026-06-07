use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::cipc::CIPCBus;
use crate::memory::HierarchicalMemoryManager;
use crate::scheduler::{CognitiveProcessContext, TokenScheduler};
use crate::security::SecurityEngine;
use crate::umi::UnifiedModelInterface;

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
    panic_log: std::sync::RwLock<Vec<PanicLog>>,
}

impl agOSKernel {
    pub fn new(umi: UnifiedModelInterface) -> Self {
        agOSKernel {
            scheduler: TokenScheduler::new(),
            memory: HierarchicalMemoryManager::new(),
            cipc: CIPCBus::new(),
            security: SecurityEngine::new(),
            umi,
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
}
