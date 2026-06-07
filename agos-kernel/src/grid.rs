use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::kernel::agOSKernel;
use crate::scheduler::CognitiveProcessContext;

pub type NodeId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PeerStatus {
    Alive,
    Suspect,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub node_id: NodeId,
    pub address: String,
    pub last_heartbeat: String,
    pub status: PeerStatus,
    pub capabilities: Vec<String>,
}

/// Serialisable snapshot of a process for cross-node migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigratingProcess {
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
    pub source_node: NodeId,
}

/// A forwarded cIPC message between grid nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridMessage {
    pub message_id: String,
    pub from: String,
    pub to: String,
    pub payload: Value,
    pub timestamp: String,
    pub source_node: NodeId,
    pub target_node: NodeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridStats {
    pub node_id: NodeId,
    pub listen_addr: String,
    pub peer_count: usize,
    pub alive_peers: usize,
    pub dead_peers: usize,
}

pub struct CognitiveGrid {
    node_id: NodeId,
    listen_addr: String,
    peers: Mutex<HashMap<NodeId, PeerInfo>>,
}

impl Default for CognitiveGrid {
    fn default() -> Self {
        Self::new("127.0.0.1:0")
    }
}

impl CognitiveGrid {
    /// Creates a new grid node with a unique UUID-based node id.
    pub fn new(listen_addr: &str) -> Self {
        let node_id = format!("node-{}", &Uuid::new_v4().to_string()[..12]);
        CognitiveGrid {
            node_id,
            listen_addr: listen_addr.to_string(),
            peers: Mutex::new(HashMap::new()),
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn listen_addr(&self) -> &str {
        &self.listen_addr
    }

    /// Register (or update) a peer node.
    pub fn add_peer(&self, node_id: NodeId, address: String) {
        let mut guard = self.peers.lock().unwrap();
        guard.insert(
            node_id.clone(),
            PeerInfo {
                node_id,
                address,
                last_heartbeat: Utc::now().to_rfc3339(),
                status: PeerStatus::Alive,
                capabilities: Vec::new(),
            },
        );
    }

    /// Mark a peer as suspected dead after missed heartbeats.
    pub fn mark_suspect(&self, node_id: &str) {
        if let Some(peer) = self.peers.lock().unwrap().get_mut(node_id) {
            peer.status = PeerStatus::Suspect;
        }
    }

    /// Remove a peer entirely.
    pub fn remove_peer(&self, node_id: &str) {
        self.peers.lock().unwrap().remove(node_id);
    }

    pub fn list_peers(&self) -> Vec<PeerInfo> {
        let mut peers: Vec<PeerInfo> = self.peers.lock().unwrap().values().cloned().collect();
        peers.sort_by(|a, b| b.last_heartbeat.cmp(&a.last_heartbeat));
        peers
    }

    /// Record a heartbeat from a peer and mark it Alive.
    pub fn record_heartbeat(&self, node_id: &str) {
        if let Some(peer) = self.peers.lock().unwrap().get_mut(node_id) {
            peer.last_heartbeat = Utc::now().to_rfc3339();
            peer.status = PeerStatus::Alive;
        }
    }

    /// Returns peers whose last heartbeat is older than the given timeout.
    pub fn stale_peers(&self, timeout_secs: i64) -> Vec<NodeId> {
        let now = Utc::now();
        let mut stale = Vec::new();
        let guard = self.peers.lock().unwrap();
        for (id, peer) in guard.iter() {
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&peer.last_heartbeat) {
                let elapsed = (now - ts.with_timezone(&Utc)).num_seconds();
                if elapsed > timeout_secs {
                    stale.push(id.clone());
                }
            }
        }
        stale
    }

    /// Returns grid statistics.
    pub fn stats(&self) -> GridStats {
        let guard = self.peers.lock().unwrap();
        let alive = guard.values().filter(|p| p.status == PeerStatus::Alive).count();
        let dead = guard.values().filter(|p| p.status == PeerStatus::Dead).count();
        GridStats {
            node_id: self.node_id.clone(),
            listen_addr: self.listen_addr.clone(),
            peer_count: guard.len(),
            alive_peers: alive,
            dead_peers: dead,
        }
    }

    // ── Agent Migration ───────────────────────────────────────

    /// Serialises a local process into a `MigratingProcess` for transfer.
    pub fn freeze_process(
        &self,
        ctx: &CognitiveProcessContext,
    ) -> MigratingProcess {
        MigratingProcess {
            pid: ctx.pid.clone(),
            manifest: ctx.manifest.clone(),
            token_quota: ctx.token_quota,
            tokens_used: ctx.tokens_used,
            priority: ctx.priority,
            active_l1_context: ctx.active_l1_context.clone(),
            state_machine: ctx.state_machine.clone(),
            parent_pid: ctx.parent_pid.clone(),
            goal_metric: ctx.goal_metric,
            created_at: ctx.created_at.clone(),
            source_node: self.node_id.clone(),
        }
    }

    /// Deserialises a `MigratingProcess` into a `CognitiveProcessContext`
    /// that the local kernel can register.
    pub fn receive_process(&self, incoming: MigratingProcess) -> CognitiveProcessContext {
        let mut ctx = CognitiveProcessContext::new(incoming.pid.clone(), incoming.manifest);
        ctx.token_quota = incoming.token_quota;
        ctx.tokens_used = incoming.tokens_used;
        ctx.priority = incoming.priority;
        ctx.active_l1_context = incoming.active_l1_context;
        ctx.state_machine = incoming.state_machine;
        ctx.parent_pid = incoming.parent_pid;
        ctx.goal_metric = incoming.goal_metric;
        ctx.created_at = incoming.created_at;
        ctx
    }

    /// Resolves a PID to its node — returns None if unknown,
    /// Some("local") if local, Some(node_id) if remote.
    pub async fn resolve_pid(
        &self,
        pid: &str,
        kernel: &agOSKernel,
    ) -> Option<String> {
        let processes = kernel.scheduler.list_processes().await;
        if processes.iter().any(|p| p.pid == pid) {
            return Some("local".to_string());
        }
        // Remote check would require querying peers; for now only local.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::umi::UnifiedModelInterface;

    fn test_kernel() -> Arc<agOSKernel> {
        Arc::new(agOSKernel::new(UnifiedModelInterface::new(), "127.0.0.1:0"))
    }

    #[test]
    fn test_node_id_generated() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        assert!(grid.node_id().starts_with("node-"));
        assert_eq!(grid.listen_addr(), "127.0.0.1:7400");
    }

    #[test]
    fn test_add_and_list_peers() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        grid.add_peer("node-abc".into(), "192.168.1.10:7400".into());
        grid.add_peer("node-def".into(), "192.168.1.11:7400".into());
        let peers = grid.list_peers();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].status, PeerStatus::Alive);
    }

    #[test]
    fn test_remove_peer() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        grid.add_peer("node-abc".into(), "192.168.1.10:7400".into());
        grid.remove_peer("node-abc");
        assert!(grid.list_peers().is_empty());
    }

    #[test]
    fn test_heartbeat_updates_status() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        grid.add_peer("node-abc".into(), "192.168.1.10:7400".into());
        grid.mark_suspect("node-abc");
        assert_eq!(
            grid.list_peers()[0].status,
            PeerStatus::Suspect
        );
        grid.record_heartbeat("node-abc");
        assert_eq!(
            grid.list_peers()[0].status,
            PeerStatus::Alive
        );
    }

    #[test]
    fn test_stale_peers_detection() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        grid.add_peer("node-old".into(), "192.168.1.10:7400".into());
        // Manually set heartbeat far in the past
        {
            let mut guard = grid.peers.lock().unwrap();
            if let Some(peer) = guard.get_mut("node-old") {
                peer.last_heartbeat = "2020-01-01T00:00:00Z".to_string();
            }
        }
        let stale = grid.stale_peers(30);
        assert!(
            stale.contains(&"node-old".to_string()),
            "should detect stale peer"
        );
    }

    #[test]
    fn test_stats() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        grid.add_peer("node-a".into(), "10.0.0.1:7400".into());
        grid.add_peer("node-b".into(), "10.0.0.2:7400".into());
        let stats = grid.stats();
        assert_eq!(stats.node_id, grid.node_id());
        assert_eq!(stats.peer_count, 2);
        assert_eq!(stats.alive_peers, 2);
    }

    #[test]
    fn test_freeze_process() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        let ctx = CognitiveProcessContext::new(
            "proc-test".into(),
            serde_json::json!({"limits": {"max_tokens": 500}}),
        );
        let frozen = grid.freeze_process(&ctx);
        assert_eq!(frozen.pid, "proc-test");
        assert_eq!(frozen.source_node, grid.node_id());
        assert_eq!(frozen.token_quota, 500);
    }

    #[tokio::test]
    async fn test_receive_process() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        let kernel = test_kernel();

        let incoming = MigratingProcess {
            pid: "proc-migrated".to_string(),
            manifest: serde_json::json!({"limits": {"max_tokens": 1000}}),
            token_quota: 1000,
            tokens_used: 100,
            priority: 2,
            active_l1_context: vec![serde_json::json!({"key": "value"})],
            state_machine: Value::Null,
            parent_pid: None,
            goal_metric: Some(0.95),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            source_node: "node-remote".to_string(),
        };

        let ctx = grid.receive_process(incoming);
        assert_eq!(ctx.pid, "proc-migrated");
        assert_eq!(ctx.tokens_used, 100);

        // Register it through the kernel
        kernel.scheduler.register(ctx).await;
        let processes = kernel.scheduler.list_processes().await;
        assert!(processes.iter().any(|p| p.pid == "proc-migrated"));
    }

    #[tokio::test]
    async fn test_resolve_pid_local() {
        let grid = CognitiveGrid::new("127.0.0.1:7400");
        let kernel = test_kernel();

        kernel.scheduler.register(
            CognitiveProcessContext::new(
                "proc-local".into(),
                serde_json::json!({}),
            ),
        ).await;

        let resolution = grid.resolve_pid("proc-local", &kernel).await;
        assert_eq!(resolution, Some("local".to_string()));

        let unknown = grid.resolve_pid("proc-unknown", &kernel).await;
        assert_eq!(unknown, None);
    }
}
