use std::sync::Arc;

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use tower_http::cors::{Any, CorsLayer};

use crate::kernel::agOSKernel;
use crate::sandbox::{AAppLoader, InstallResult};

type KernelState = Arc<agOSKernel>;

pub fn create_router(kernel: Arc<agOSKernel>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health_check))
        .route("/kernel/spawn", post(kernel_spawn))
        .route("/kernel/kill", post(kernel_kill))
        .route("/kernel/processes", get(kernel_processes))
        .route("/kernel/process/{pid}", get(kernel_process))
        .route("/kernel/panic", post(kernel_panic))
        .route("/memory/l1/write", post(memory_l1_write))
        .route("/memory/l1/{pid}", get(memory_l1_read))
        .route("/memory/l2/compress", post(memory_l2_compress))
        .route("/memory/vector", post(memory_vector_store))
        .route("/memory/page", post(memory_page_in))
        .route("/memory/stats", get(memory_stats))
        .route("/memory/episodic", get(memory_episodic))
        .route("/cipc/route", post(cipc_route))
        .route("/cipc/history", get(cipc_history))
        .route("/security/verify", post(security_verify))
        .route("/security/audit", get(security_audit))
        .route("/umi/reason", post(umi_reason))
        .route("/umi/embed", post(umi_embed))
        .route("/umi/compress", post(umi_compress))
        .route("/umi/switch", post(umi_switch))
        .route("/umi/metrics", get(umi_metrics))
        .route("/sandbox/install", post(sandbox_install))
        .route("/sandbox/launch", post(sandbox_launch))
        .route("/sandbox/apps", get(sandbox_apps))
        .layer(cors)
        .with_state(kernel)
}

async fn health_check() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok",
        "system": "agOS Kernel v0.1.0"
    }))
}

// ── Kernel ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct SpawnRequest {
    manifest: Value,
    parent_pid: Option<String>,
}

async fn kernel_spawn(
    State(kernel): State<KernelState>,
    Json(req): Json<SpawnRequest>,
) -> Json<Value> {
    let ctx = kernel.spawn_process(req.manifest, req.parent_pid).await;
    Json(serde_json::json!({
        "pid": ctx.pid,
        "priority": ctx.priority,
        "token_quota": ctx.token_quota,
        "goal_metric": ctx.goal_metric,
    }))
}

#[derive(Deserialize)]
struct KillRequest {
    pid: String,
}

async fn kernel_kill(
    State(kernel): State<KernelState>,
    Json(req): Json<KillRequest>,
) -> Json<Value> {
    kernel.kill_process(&req.pid).await;
    Json(serde_json::json!({"ok": true, "pid": req.pid}))
}

async fn kernel_processes(State(kernel): State<KernelState>) -> Json<Value> {
    let procs = kernel.scheduler.list_processes().await;
    Json(serde_json::json!({"processes": procs}))
}

async fn kernel_process(
    State(kernel): State<KernelState>,
    Path(pid): Path<String>,
) -> Json<Value> {
    match kernel.scheduler.get_process(&pid).await {
        Some(ctx) => Json(serde_json::json!({"process": ctx})),
        None => Json(serde_json::json!({"error": "Process not found", "pid": pid})),
    }
}

#[derive(Deserialize)]
struct PanicRequest {
    pid: String,
    reason: String,
}

async fn kernel_panic(
    State(kernel): State<KernelState>,
    Json(req): Json<PanicRequest>,
) -> Json<Value> {
    let entry = kernel.kernel_panic(&req.pid, &req.reason).await;
    Json(serde_json::json!({
        "panic": entry,
        "alert": format!("Broadcast sent for PID {}", req.pid),
        "l1_flushed": true,
    }))
}

// ── Memory ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct L1WriteRequest {
    pid: String,
    data: Vec<Value>,
}

async fn memory_l1_write(
    State(kernel): State<KernelState>,
    Json(req): Json<L1WriteRequest>,
) -> Json<Value> {
    kernel.memory.write_l1(&req.pid, req.data);
    Json(serde_json::json!({"ok": true}))
}

async fn memory_l1_read(
    State(kernel): State<KernelState>,
    Path(pid): Path<String>,
) -> Json<Value> {
    let data = kernel.memory.read_l1(&pid);
    Json(serde_json::json!({"pid": pid, "data": data}))
}

#[derive(Deserialize)]
struct L2CompressRequest {
    pid: String,
}

async fn memory_l2_compress(
    State(kernel): State<KernelState>,
    Json(req): Json<L2CompressRequest>,
) -> Json<Value> {
    let compressed = kernel.memory.compress_to_l2(&req.pid);
    Json(serde_json::json!({"pid": req.pid, "compressed": compressed}))
}

#[derive(Deserialize)]
struct VectorStoreRequest {
    vector: Vec<f64>,
    metadata: Value,
}

async fn memory_vector_store(
    State(kernel): State<KernelState>,
    Json(req): Json<VectorStoreRequest>,
) -> Json<Value> {
    kernel.memory.store_vector(req.vector, req.metadata);
    Json(serde_json::json!({"ok": true}))
}

#[derive(Deserialize)]
struct PageInRequest {
    query_vector: Vec<f64>,
    top_k: Option<usize>,
}

async fn memory_page_in(
    State(kernel): State<KernelState>,
    Json(req): Json<PageInRequest>,
) -> Json<Value> {
    let pages = kernel.memory.page_in(&req.query_vector, req.top_k.unwrap_or(3));
    Json(serde_json::json!({"pages": pages, "count": pages.len()}))
}

async fn memory_stats(State(kernel): State<KernelState>) -> Json<Value> {
    let stats = kernel.memory.get_stats();
    Json(serde_json::json!({"stats": stats}))
}

async fn memory_episodic(State(kernel): State<KernelState>) -> Json<Value> {
    let entries = kernel.memory.get_episodic_log(None, 50);
    Json(serde_json::json!({"entries": entries, "count": entries.len()}))
}

// ── cIPC ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CipcRouteRequest {
    from: String,
    to: String,
    payload: Value,
}

async fn cipc_route(
    State(kernel): State<KernelState>,
    Json(req): Json<CipcRouteRequest>,
) -> Json<Value> {
    match kernel.cipc.route(&req.from, &req.to, req.payload) {
        Ok(msg) => Json(serde_json::json!({"ok": true, "message_id": msg.id})),
        Err(e) => Json(serde_json::json!({"ok": false, "error": e})),
    }
}

async fn cipc_history(State(kernel): State<KernelState>) -> Json<Value> {
    let msgs = kernel.cipc.get_history(None, 50);
    Json(serde_json::json!({"messages": msgs, "count": msgs.len()}))
}

// ── Security ────────────────────────────────────────────────

#[derive(Deserialize)]
struct VerifyRequest {
    action_vector: Value,
    permissions: Vec<String>,
}

async fn security_verify(
    State(kernel): State<KernelState>,
    Json(req): Json<VerifyRequest>,
) -> Json<Value> {
    match kernel.security.ring0_verify(&req.action_vector, &req.permissions) {
        Ok(()) => Json(serde_json::json!({"allowed": true, "reason": "verified"})),
        Err(e) => Json(serde_json::json!({"allowed": false, "reason": e})),
    }
}

async fn security_audit(State(kernel): State<KernelState>) -> Json<Value> {
    let entries = kernel.security.get_audit_log(50);
    let allowed = entries.iter().filter(|e| e.result == "ALLOWED").count();
    let denied = entries.iter().filter(|e| e.result == "DENIED").count();
    Json(serde_json::json!({"entries": entries, "count": entries.len(), "allowed": allowed, "denied": denied}))
}

// ── UMI ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ReasonRequest {
    prompt_vector: Vec<String>,
}

async fn umi_reason(
    State(kernel): State<KernelState>,
    Json(req): Json<ReasonRequest>,
) -> Json<Value> {
    let result = kernel.umi.reason(&req.prompt_vector, &Value::Null).await;
    Json(serde_json::json!({
        "output": result.output,
        "tokens_used": result.tokens_used,
        "model": result.model,
        "mode": result.mode,
    }))
}

#[derive(Deserialize)]
struct EmbedRequest {
    data: Vec<String>,
}

async fn umi_embed(
    State(kernel): State<KernelState>,
    Json(req): Json<EmbedRequest>,
) -> Json<Value> {
    let embedding = kernel.umi.embed(&req.data);
    Json(serde_json::json!({"embedding": embedding, "driver": kernel.umi.active_name()}))
}

#[derive(Deserialize)]
struct CompressRequest {
    data: Vec<String>,
}

async fn umi_compress(
    State(kernel): State<KernelState>,
    Json(req): Json<CompressRequest>,
) -> Json<Value> {
    let compressed = kernel.umi.compress(&req.data);
    Json(serde_json::json!({"compressed": compressed, "driver": kernel.umi.active_name()}))
}

#[derive(Deserialize)]
struct SwitchDriverRequest {
    driver_name: String,
}

async fn umi_switch(
    State(kernel): State<KernelState>,
    Json(req): Json<SwitchDriverRequest>,
) -> Json<Value> {
    match kernel.hot_swap_umi(&req.driver_name) {
        Ok(()) => Json(serde_json::json!({
            "ok": true,
            "active_driver": kernel.umi.active_name(),
            "capability": kernel.umi.active_capability(),
        })),
        Err(e) => Json(serde_json::json!({"ok": false, "error": e})),
    }
}

async fn umi_metrics(State(kernel): State<KernelState>) -> Json<Value> {
    let metrics = kernel.umi.get_metrics();
    Json(serde_json::json!({
        "metrics": metrics,
        "active_driver": kernel.umi.active_name(),
    }))
}

// ── Sandbox ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct InstallRequest {
    name: String,
    aapps_path: Option<String>,
}

async fn sandbox_install(
    State(kernel): State<KernelState>,
    Json(req): Json<InstallRequest>,
) -> Json<Value> {
    let base = req.aapps_path.unwrap_or_else(|| "aapps".to_string());
    let path = std::path::Path::new(&base).join(&req.name);

    let aapp = match AAppLoader::load_from_path(&path) {
        Ok(pkg) => pkg,
        Err(e) => return Json(serde_json::json!({"error": e})),
    };

    let version = aapp
        .manifest
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let permissions: Vec<String> = aapp
        .manifest
        .get("permissions")
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let token_quota = aapp
        .manifest
        .get("limits")
        .and_then(|l| l.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1000);

    for perm in &permissions {
        let action_vector = serde_json::json!({
            "action": perm,
            "capability": perm,
            "target": "installer",
        });
        if let Err(e) = kernel.security.ring0_verify(&action_vector, &["*:*".to_string()]) {
            return Json(serde_json::json!({
                "error": format!("Permission '{}' denied by Ring 0: {}", perm, e)
            }));
        }
    }

    let states = aapp
        .logic_graphs
        .get("states")
        .and_then(|s| s.as_object())
        .map(|o| o.len())
        .unwrap_or(0);

    kernel.memory.log_episodic(
        "sandbox",
        "APP_INSTALLED",
        serde_json::json!({"name": &aapp.name, "version": &version}),
        None,
    );

    Json(serde_json::json!(InstallResult {
        status: "installed".to_string(),
        name: aapp.name.clone(),
        version,
        permissions,
        token_quota,
        states,
    }))
}

#[derive(Deserialize)]
struct LaunchRequest {
    name: String,
    parent_pid: Option<String>,
}

async fn sandbox_launch(
    State(kernel): State<KernelState>,
    Json(req): Json<LaunchRequest>,
) -> Json<Value> {
    let base = "aapps".to_string();
    let path = std::path::Path::new(&base).join(&req.name);

    let aapp = match AAppLoader::load_from_path(&path) {
        Ok(pkg) => pkg,
        Err(e) => return Json(serde_json::json!({"error": e})),
    };

    let ctx = kernel.spawn_process(aapp.manifest, req.parent_pid).await;
    kernel.cipc.register_process(&ctx.pid, vec![]);

    kernel.memory.log_episodic(
        &ctx.pid,
        "APP_LAUNCHED",
        serde_json::json!({"app": &aapp.name, "logic_graphs": aapp.logic_graphs}),
        None,
    );

    Json(serde_json::json!({
        "pid": ctx.pid,
        "priority": ctx.priority,
        "token_quota": ctx.token_quota,
    }))
}

async fn sandbox_apps() -> Json<Value> {
    Json(serde_json::json!({"apps": ["financial_auditor", "accountant"]}))
}
