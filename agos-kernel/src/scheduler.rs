use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::ModelRegistry;

#[async_trait::async_trait]
pub trait SchedulePolicy: Send + Sync {
    fn name(&self) -> &'static str;
    fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize>;
    fn allocate_tokens(
        &self,
        ctx: &CognitiveProcessContext,
        complexity: f64,
    ) -> Result<u64, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveProcessContext {
    pub pid: String,
    pub manifest: Value,
    pub token_quota: u64,
    pub tokens_used: u64,
    pub priority: u32,
    pub model: String,
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
        let model = manifest
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gpt-4")
            .to_string();

        CognitiveProcessContext {
            pid,
            manifest,
            token_quota: limits,
            tokens_used: 0,
            priority,
            model,
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

pub struct PriorityPreemption;

#[async_trait::async_trait]
impl SchedulePolicy for PriorityPreemption {
    fn name(&self) -> &'static str {
        "priority_preemption"
    }

    fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize> {
        let mut best = None;
        let mut best_priority = u32::MAX;
        for (i, p) in processes.iter().enumerate() {
            if !p.is_exhausted() && p.priority < best_priority {
                best = Some(i);
                best_priority = p.priority;
            }
        }
        best
    }

    fn allocate_tokens(
        &self,
        ctx: &CognitiveProcessContext,
        complexity: f64,
    ) -> Result<u64, String> {
        let weight = if ctx.priority > 1 { 1.0 } else { 0.3 };
        let allocation = (complexity * weight) as u64;
        Ok(allocation.max(1))
    }
}

pub struct RoundRobin;

#[async_trait::async_trait]
impl SchedulePolicy for RoundRobin {
    fn name(&self) -> &'static str {
        "round_robin"
    }

    fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize> {
        let active: Vec<usize> = processes
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_exhausted())
            .map(|(i, _)| i)
            .collect();
        if active.is_empty() {
            None
        } else {
            Some(active[0])
        }
    }

    fn allocate_tokens(
        &self,
        _ctx: &CognitiveProcessContext,
        complexity: f64,
    ) -> Result<u64, String> {
        let allocation = complexity as u64;
        Ok(allocation.max(1))
    }
}

pub struct FIFO;

#[async_trait::async_trait]
impl SchedulePolicy for FIFO {
    fn name(&self) -> &'static str {
        "fifo"
    }

    fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize> {
        processes
            .iter()
            .enumerate()
            .find(|(_, p)| !p.is_exhausted())
            .map(|(i, _)| i)
    }

    fn allocate_tokens(
        &self,
        _ctx: &CognitiveProcessContext,
        complexity: f64,
    ) -> Result<u64, String> {
        let allocation = complexity as u64;
        Ok(allocation.max(1))
    }
}

pub struct TokenAware;

#[async_trait::async_trait]
impl SchedulePolicy for TokenAware {
    fn name(&self) -> &'static str {
        "token_aware"
    }

    fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize> {
        let mut best = None;
        let mut best_remaining = 0f64;
        for (i, p) in processes.iter().enumerate() {
            if p.is_exhausted() {
                continue;
            }
            let remaining = if p.token_quota > 0 {
                (p.token_quota - p.tokens_used) as f64 / p.token_quota as f64
            } else {
                0.0
            };
            if remaining > best_remaining {
                best = Some(i);
                best_remaining = remaining;
            }
        }
        best
    }

    fn allocate_tokens(
        &self,
        _ctx: &CognitiveProcessContext,
        complexity: f64,
    ) -> Result<u64, String> {
        let allocation = (complexity * 0.5) as u64;
        Ok(allocation.max(1))
    }
}

pub struct TokenScheduler {
    processes: Arc<RwLock<Vec<CognitiveProcessContext>>>,
    policy: RwLock<Box<dyn SchedulePolicy>>,
    pub model_registry: Arc<ModelRegistry>,
}

impl Default for TokenScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenScheduler {
    pub fn new() -> Self {
        TokenScheduler {
            processes: Arc::new(RwLock::new(Vec::new())),
            policy: RwLock::new(Box::new(PriorityPreemption)),
            model_registry: Arc::new(ModelRegistry::builtin()),
        }
    }

    pub fn new_with_policy(policy: Box<dyn SchedulePolicy>) -> Self {
        TokenScheduler {
            processes: Arc::new(RwLock::new(Vec::new())),
            policy: RwLock::new(policy),
            model_registry: Arc::new(ModelRegistry::builtin()),
        }
    }

    pub async fn policy_name(&self) -> &'static str {
        self.policy.read().await.name()
    }

    pub async fn set_policy(&self, policy: Box<dyn SchedulePolicy>) {
        *self.policy.write().await = policy;
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
        procs.iter_mut().find(|p| p.pid == pid).map(f)
    }

    pub async fn allocate_inference(
        &self,
        pid: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<u64, String> {
        let model_name = {
            let procs = self.processes.read().await;
            procs
                .iter()
                .find(|p| p.pid == pid)
                .ok_or_else(|| format!("Unknown PID: {}", pid))?
                .model
                .clone()
        };

        let cost = self
            .model_registry
            .cost_for(&model_name, input_tokens, output_tokens)
            .ok_or_else(|| format!("Unknown model: {}", model_name))?;

        let mut procs = self.processes.write().await;
        if let Some(ctx) = procs.iter_mut().find(|p| p.pid == pid) {
            ctx.consume_tokens(cost)?;
        }
        Ok(cost)
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
