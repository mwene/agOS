use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveProcessContext {
    pub pid: String,
    pub manifest: Value,
    pub token_quota: u64,
    pub tokens_used: u64,
    pub priority: u32,
    pub active_l1_context: Vec<Value>,
    pub state_machine: Value,
    pub parent_pid: Option<String>,
    pub goal_metric: Option<f64>,
    pub created_at: String,
}

impl CognitiveProcessContext {
    pub fn new(pid: String, manifest: Value) -> Self {
        let limits = manifest
            .get("limits")
            .and_then(|l| l.get("max_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        let priority = manifest
            .get("priority")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let goal = manifest
            .get("goal_metric")
            .and_then(|v| v.as_f64());

        CognitiveProcessContext {
            pid,
            manifest,
            token_quota: limits,
            tokens_used: 0,
            priority,
            active_l1_context: Vec::new(),
            state_machine: Value::Null,
            parent_pid: None,
            goal_metric: goal,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    pub fn consume_tokens(&mut self, count: u64) -> Result<(), String> {
        self.tokens_used += count;
        if self.tokens_used > self.token_quota {
            Err(format!(
                "PID {}: Token quota {} exhausted (used {})",
                self.pid, self.token_quota, self.tokens_used
            ))
        } else {
            Ok(())
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.tokens_used >= self.token_quota
    }

    pub fn save_state(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerStats {
    pub total_processes: usize,
    pub system1_count: usize,
    pub system2_count: usize,
}

pub struct TokenScheduler {
    processes: Arc<RwLock<Vec<CognitiveProcessContext>>>,
}

impl TokenScheduler {
    pub fn new() -> Self {
        TokenScheduler {
            processes: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn register(&self, ctx: CognitiveProcessContext) {
        let mut procs = self.processes.write().await;
        procs.push(ctx);
    }

    pub async fn unregister(&self, pid: &str) {
        let mut procs = self.processes.write().await;
        procs.retain(|p| p.pid != pid);
    }

    pub async fn get_process(&self, pid: &str) -> Option<CognitiveProcessContext> {
        let procs = self.processes.read().await;
        procs.iter().find(|p| p.pid == pid).cloned()
    }

    pub async fn get_process_mut<F, R>(&self, pid: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut CognitiveProcessContext) -> R,
    {
        let mut procs = self.processes.write().await;
        if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
            Some(f(p))
        } else {
            None
        }
    }

    pub async fn allocate_inference(
        &self,
        pid: &str,
        complexity: f64,
    ) -> Result<u64, String> {
        let weight = {
            let procs = self.processes.read().await;
            let ctx = procs.iter().find(|p| p.pid == pid)
                .ok_or_else(|| format!("Unknown PID: {}", pid))?;
            if ctx.priority > 1 { 1.0 } else { 0.3 }
        };
        let allocation = (complexity * weight) as u64;
        let mut procs = self.processes.write().await;
        if let Some(ctx) = procs.iter_mut().find(|p| p.pid == pid) {
            ctx.consume_tokens(allocation)?;
        }
        Ok(allocation)
    }

    pub async fn list_processes(&self) -> Vec<CognitiveProcessContext> {
        self.processes.read().await.clone()
    }

    pub async fn stats(&self) -> SchedulerStats {
        let procs = self.processes.read().await;
        let system1 = procs.iter().filter(|p| p.priority <= 1).count();
        let system2 = procs.iter().filter(|p| p.priority > 1).count();
        SchedulerStats {
            total_processes: procs.len(),
            system1_count: system1,
            system2_count: system2,
        }
    }
}
