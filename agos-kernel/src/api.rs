use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use tower_http::cors::{Any, CorsLayer};
use tracing::instrument;

use crate::audit_export::{JsonlExporter, StdoutExporter};
use crate::kernel::agOSKernel;
use crate::orchestrator::WorkflowDefinition;
use crate::sandbox::{AAppLoader, InstallResult};

type KernelState = Arc<agOSKernel>;

fn check_rate_limit(kernel: &agOSKernel, key: &str) -> Result<(), Json<Value>> {
    kernel.rate_limiter.check(key).map_err(|e| {
        Json(serde_json::json!({"ok": false, "error": e}))
    })
}

fn require_auth(kernel: &agOSKernel, headers: &HeaderMap) -> Result<(), (axum::http::StatusCode, Json<Value>)> {
    let auth = kernel.auth.lock().unwrap();
    if auth.api_key.is_none() {
        return Ok(());
    }
    let api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok());
    if auth.check_api_key(api_key).is_err() {
        return Err((
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Missing or invalid API key"})),
        ));
    }
    Ok(())
}

pub fn create_router(kernel: Arc<agOSKernel>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/health", get(health_check))
        .route("/models", get(list_models))
        .route("/kernel/spawn", post(kernel_spawn))
        .route("/kernel/kill", post(kernel_kill))
        .route("/kernel/processes", get(kernel_processes))
        .route("/kernel/process/:pid", get(kernel_process))
        .route("/kernel/panic", post(kernel_panic))
        .route("/scheduler/policy", post(scheduler_set_policy))
        .route("/scheduler/policy", get(scheduler_get_policy))
        .route("/memory/l1/write", post(memory_l1_write))
        .route("/memory/l1/:pid", get(memory_l1_read))
        .route("/memory/l2/compress", post(memory_l2_compress))
        .route("/memory/vector", post(memory_vector_store))
        .route("/memory/page", post(memory_page_in))
        .route("/memory/stats", get(memory_stats))
        .route("/memory/episodic", get(memory_episodic))
        .route("/memory/episodic/compress", post(memory_episodic_compress))
        .route("/cipc/route", post(cipc_route))
        .route("/cipc/history", get(cipc_history))
        .route("/security/verify", post(security_verify))
        .route("/security/audit", get(security_audit))
        .route("/security/auth", post(security_auth_set))
        .route("/security/auth/agent", post(security_auth_agent_token))
        .route("/umi/reason", post(umi_reason))
        .route("/umi/embed", post(umi_embed))
        .route("/umi/compress", post(umi_compress))
        .route("/umi/switch", post(umi_switch))
        .route("/umi/metrics", get(umi_metrics))
        .route("/umi/drivers", get(umi_drivers))
        .route("/umi/driver/info/:name", get(umi_driver))
        .route("/umi/driver/download/:name", get(umi_driver_download))
        .route("/umi/driver/config/:name", post(umi_driver_config))
        .route("/umi/driver/register", post(umi_driver_register))
        .route("/umi/driver/unregister", post(umi_driver_unregister))
        .route("/sandbox/install", post(sandbox_install))
        .route("/sandbox/launch", post(sandbox_launch))
        .route("/sandbox/apps", get(sandbox_apps));

    #[cfg(feature = "wasm-sandbox")]
    let router = router.route("/sandbox/execute", post(sandbox_wasm_execute));

    #[cfg(feature = "embodiment")]
    let router = router
        .route("/embodiment/goal", post(embodiment_goal))
        .route("/embodiment/goals", get(embodiment_goals))
        .route("/embodiment/goal/:goal_id", get(embodiment_goal_status))
        .route("/embodiment/goal/:goal_id/plan", get(embodiment_goal_plan))
        .route("/embodiment/goal/:goal_id/clarify", post(embodiment_clarify))
        .route("/embodiment/goal/:goal_id/execute", post(embodiment_execute))
        .route("/embodiment/sensors", get(embodiment_sensors))
        .route("/embodiment/apps", get(embodiment_apps));

    #[cfg(feature = "uri")]
    let router = router
        .route("/uri/sense", post(uri_sense))
        .route("/uri/actuate", post(uri_actuate))
        .route("/uri/transform", post(uri_transform))
        .route("/uri/status", post(uri_status))
        .route("/uri/switch", post(uri_switch))
        .route("/uri/metrics", get(uri_metrics))
        .route("/uri/drivers", get(uri_drivers))
        .route("/uri/driver/:name", get(uri_driver))
        .route("/uri/driver/register", post(uri_driver_register))
        .route("/uri/driver/unregister", post(uri_driver_unregister))
        .route("/controller/pid/create", post(controller_pid_create))
        .route("/controller/pid/list", get(controller_pid_list))
        .route("/controller/pid/setpoint", post(controller_pid_setpoint))
        .route("/controller/pid/step", post(controller_pid_step))
        .route("/controller/trajectory/create", post(controller_trajectory_create))
        .route("/controller/trajectory/start", post(controller_trajectory_start))
        .route("/controller/trajectory/step", post(controller_trajectory_step))
        .route("/controller/trajectory/list", get(controller_trajectory_list))
        .route("/controller/safety/create", post(controller_safety_create))
        .route("/controller/safety/check", post(controller_safety_check))
        .route("/controller/safety/estop", post(controller_safety_estop))
        .route("/controller/safety/estop/clear", post(controller_safety_estop_clear))
        .route("/controller/safety/monitors", get(controller_safety_monitors))
        .route("/controller/safety/events/:device_id", get(controller_safety_events))
        .route("/controller/transform/add", post(controller_transform_add))
        .route("/controller/transform/list", get(controller_transform_list))
        .route("/controller/stats", get(controller_stats));

    let router = router
        .route("/security/audit/export", post(security_audit_export))
        .route("/workflow/submit", post(workflow_submit))
        .route("/workflow/execute", post(workflow_execute))
        .route("/workflow/list", get(workflow_list))
        .route("/workflow/:id", get(workflow_get))
        .route("/grid/join", post(grid_join))
        .route("/grid/peers", get(grid_peers))
        .route("/grid/stats", get(grid_stats))
        .route("/grid/heartbeat", post(grid_heartbeat))
        .route("/grid/migrate", post(grid_migrate))
        .route("/grid/receive", post(grid_receive))
        .route("/grid/cipc/forward", post(grid_cipc_forward));

    router.layer(cors).with_state(kernel)
}

#[instrument]
async fn health_check() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok",
        "system": "agOS Kernel v0.1.0"
    }))
}

// ── Models ───────────────────────────────────────────────────

async fn list_models(
    State(kernel): State<KernelState>,
) -> Json<Value> {
    let models = kernel.list_models();
    Json(serde_json::json!({
        "models": models
    }))
}

// ── Kernel ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SpawnRequest {
    manifest: Value,
    parent_pid: Option<String>,
}

#[instrument(skip(kernel))]
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

#[derive(Debug, Deserialize)]
struct KillRequest {
    pid: String,
}

#[instrument(skip(kernel))]
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

// ── Scheduler ────────────────────────────────────────────────

#[derive(Deserialize)]
struct SetPolicyRequest {
    policy: String,
}

async fn scheduler_get_policy(
    State(kernel): State<KernelState>,
) -> Json<Value> {
    let policy = kernel.scheduler_policy_name().await;
    Json(serde_json::json!({
        "policy": policy
    }))
}

async fn scheduler_set_policy(
    State(kernel): State<KernelState>,
    Json(req): Json<SetPolicyRequest>,
) -> Json<Value> {
    match kernel.set_scheduler_policy(&req.policy).await {
        Ok(()) => {
            let policy = kernel.scheduler_policy_name().await;
            Json(serde_json::json!({
                "status": "ok",
                "policy": policy
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "error": e
        })),
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
    if let Err(resp) = check_rate_limit(&kernel, &format!("pid:{}", req.pid)) {
        return resp;
    }
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

async fn memory_episodic_compress(State(kernel): State<KernelState>) -> Json<Value> {
    let count = kernel.memory.compress_episodic();
    Json(serde_json::json!({"ok": true, "compressed": count}))
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
    if let Err(resp) = check_rate_limit(&kernel, &format!("pid:{}", req.from)) {
        return resp;
    }
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

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    action_vector: Value,
    permissions: Vec<String>,
}

#[instrument(skip(kernel))]
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

#[derive(Deserialize)]
struct SecurityAuthRequest {
    api_key: String,
}

async fn security_auth_set(
    State(kernel): State<KernelState>,
    Json(req): Json<SecurityAuthRequest>,
) -> Json<Value> {
    let mut auth = kernel.auth.lock().unwrap();
    if req.api_key.is_empty() {
        auth.api_key = None;
        drop(auth);
        return Json(serde_json::json!({"ok": true, "message": "API key authentication disabled."}));
    }
    auth.api_key = Some(req.api_key.clone());
    drop(auth);
    Json(serde_json::json!({
        "ok": true,
        "message": "API key authentication enabled. All endpoints except /health and /security/auth are now protected.",
    }))
}

#[derive(Deserialize)]
struct SecurityAgentTokenRequest {
    agent_id: String,
    token: String,
}

async fn security_auth_agent_token(
    State(kernel): State<KernelState>,
    Json(req): Json<SecurityAgentTokenRequest>,
) -> Json<Value> {
    let mut auth = kernel.auth.lock().unwrap();
    auth.agent_tokens.insert(req.agent_id.clone(), req.token.clone());
    drop(auth);
    Json(serde_json::json!({"ok": true, "agent_id": req.agent_id}))
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
    if let Err(resp) = check_rate_limit(&kernel, "system:umi") {
        return resp;
    }
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
    if let Err(resp) = check_rate_limit(&kernel, "system:umi") {
        return resp;
    }
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

async fn umi_drivers(State(kernel): State<KernelState>) -> Json<Value> {
    let drivers = kernel.umi.list_drivers();
    Json(serde_json::json!({ "drivers": drivers, "count": drivers.len() }))
}

async fn umi_driver(
    State(kernel): State<KernelState>,
    Path(name): Path<String>,
) -> Json<Value> {
    match kernel.umi.get_driver(&name) {
        Some(info) => Json(serde_json::json!(info)),
        None => Json(serde_json::json!({"error": format!("Driver '{}' not found", name)})),
    }
}

#[derive(Deserialize)]
struct RegisterDriverRequest {
    name: String,
    capability_level: u32,
    #[serde(default = "default_token_cost")]
    token_cost: u64,
    #[serde(default = "default_prefix")]
    prefix: String,
    #[serde(default = "default_embed_modulus")]
    embed_modulus: u64,
    #[serde(default = "default_max_output")]
    max_output: usize,
    #[serde(default)]
    config: HashMap<String, String>,
}

fn default_token_cost() -> u64 { 1 }
fn default_prefix() -> String { "inferred".to_string() }
fn default_embed_modulus() -> u64 { 10000 }
fn default_max_output() -> usize { 10 }

async fn umi_driver_register(
    State(kernel): State<KernelState>,
    Json(req): Json<RegisterDriverRequest>,
) -> Json<Value> {
    match kernel.umi.register_driver(
        &req.name, req.capability_level, req.token_cost, &req.prefix,
        req.embed_modulus, req.max_output, req.config,
    ) {
        Ok(()) => Json(serde_json::json!({"ok": true, "name": req.name})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn umi_driver_unregister(
    State(kernel): State<KernelState>,
    Json(req): Json<UnregisterDriverRequest>,
) -> Json<Value> {
    match kernel.umi.unregister_driver(&req.name) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn umi_driver_download(
    State(kernel): State<KernelState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Json<Value> {
    if let Err((_status, body)) = require_auth(&kernel, &headers) {
        return body;
    }
    let info = kernel.umi.get_driver(&name);
    let config = kernel.umi.get_driver_config(&name);
    match info {
        Some(info) => Json(serde_json::json!({
            "driver": info,
            "config": config.unwrap_or_default(),
        })),
        None => Json(serde_json::json!({"error": format!("Driver '{}' not found", name)})),
    }
}

#[derive(Deserialize)]
struct UpdateDriverConfigRequest {
    config: HashMap<String, String>,
}

async fn umi_driver_config(
    State(kernel): State<KernelState>,
    Path(name): Path<String>,
    Json(req): Json<UpdateDriverConfigRequest>,
) -> Json<Value> {
    match kernel.umi.update_driver_config(&name, req.config) {
        Ok(()) => Json(serde_json::json!({"ok": true, "name": name})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[derive(Deserialize)]
struct UnregisterDriverRequest {
    name: String,
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

    let has_workflow = aapp.workflow.is_some();
    let signature_valid = aapp.signature_valid;

    if signature_valid == Some(false) {
        return Json(serde_json::json!({
            "error": "Signature verification failed for .aapp package",
            "name": aapp.name,
        }));
    }

    Json(serde_json::json!(InstallResult {
        status: "installed".to_string(),
        name: aapp.name.clone(),
        version,
        permissions,
        token_quota,
        states,
        signature_valid,
        has_workflow,
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

#[cfg(feature = "wasm-sandbox")]
#[derive(Deserialize)]
struct WasmExecuteRequest {
    wasm_base64: String,
    input: Value,
    pid: String,
    fuel_quota: Option<u64>,
}

#[cfg(feature = "wasm-sandbox")]
async fn sandbox_wasm_execute(
    State(kernel): State<KernelState>,
    Json(req): Json<WasmExecuteRequest>,
) -> Json<Value> {
    use base64::Engine as _;

    let wasm_bytes = match base64::engine::general_purpose::STANDARD.decode(&req.wasm_base64) {
        Ok(b) => b,
        Err(e) => return Json(serde_json::json!({"error": format!("Base64 decode: {}", e)})),
    };

    let sandbox = match crate::sandbox_wasm::WasmSandbox::new() {
        Ok(s) => s,
        Err(e) => return Json(serde_json::json!({"error": e})),
    };

    let quota = req.fuel_quota.unwrap_or(100_000);
    match sandbox.execute_tool(&wasm_bytes, &req.input, &kernel, &req.pid, quota) {
        Ok(result) => Json(serde_json::json!({"ok": true, "result": result})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

// ── Embodiment (VLA Driver Pipeline) ────────────────────────

#[cfg(feature = "embodiment")]
#[derive(Deserialize)]
struct EmbodimentGoalRequest {
    statement: String,
    sensory_inputs: Option<Vec<EmbodimentSensoryInput>>,
}

#[cfg(feature = "embodiment")]
#[derive(Deserialize)]
struct EmbodimentSensoryInput {
    modality: String,
    data: String,
    metadata: Option<Value>,
}

#[cfg(feature = "embodiment")]
#[derive(Deserialize)]
struct EmbodimentClarifyRequest {
    clarification: String,
}

#[cfg(feature = "embodiment")]
async fn embodiment_goal(
    State(kernel): State<KernelState>,
    Json(req): Json<EmbodimentGoalRequest>,
) -> Json<Value> {
    use crate::embodiment::SenseModality;

    let sensory_inputs = match req.sensory_inputs {
        Some(inputs) => inputs
            .into_iter()
            .filter_map(|si| {
                let modality = match si.modality.to_lowercase().as_str() {
                    "text" => Some(SenseModality::Text),
                    "image" => Some(SenseModality::Image),
                    "audio" => Some(SenseModality::Audio),
                    "video" => Some(SenseModality::Video),
                    "haptic" => Some(SenseModality::Haptic),
                    "olfactory" => Some(SenseModality::Olfactory),
                    "gustatory" => Some(SenseModality::Gustatory),
                    _ => None,
                }?;
                Some(kernel.embodiment.process_sensory_input(
                    modality,
                    &si.data,
                    si.metadata.unwrap_or(Value::Null),
                ))
            })
            .collect(),
        None => Vec::new(),
    };

    let goal = kernel
        .embodiment
        .submit_goal(&req.statement, sensory_inputs, &kernel);

    let plan = kernel
        .embodiment
        .run_pipeline(&goal.id, &kernel)
        .await
        .ok();

    Json(serde_json::json!({
        "goal": goal,
        "plan": plan,
    }))
}

#[cfg(feature = "embodiment")]
async fn embodiment_goals(State(kernel): State<KernelState>) -> Json<Value> {
    let goals = kernel.embodiment.list_goals();
    Json(serde_json::json!({"goals": goals, "count": goals.len()}))
}

#[cfg(feature = "embodiment")]
async fn embodiment_goal_status(
    State(kernel): State<KernelState>,
    Path(goal_id): Path<String>,
) -> Json<Value> {
    match kernel.embodiment.get_goal(&goal_id) {
        Some(goal) => Json(serde_json::json!({"goal": goal})),
        None => Json(serde_json::json!({"error": "Goal not found"})),
    }
}

#[cfg(feature = "embodiment")]
async fn embodiment_goal_plan(
    State(kernel): State<KernelState>,
    Path(goal_id): Path<String>,
) -> Json<Value> {
    match kernel.embodiment.get_plan(&goal_id) {
        Some(plan) => Json(serde_json::json!({"plan": plan})),
        None => Json(serde_json::json!({"error": "No plan found for goal"})),
    }
}

#[cfg(feature = "embodiment")]
async fn embodiment_clarify(
    State(kernel): State<KernelState>,
    Path(goal_id): Path<String>,
    Json(req): Json<EmbodimentClarifyRequest>,
) -> Json<Value> {
    match kernel
        .embodiment
        .provide_clarification(&goal_id, &req.clarification, &kernel)
        .await
    {
        Ok(plan) => Json(serde_json::json!({"ok": true, "plan": plan})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "embodiment")]
async fn embodiment_execute(
    State(kernel): State<KernelState>,
    Path(goal_id): Path<String>,
) -> Json<Value> {
    match kernel.embodiment.execute_plan(&goal_id, &kernel).await {
        Ok(()) => {
            let goal = kernel.embodiment.get_goal(&goal_id);
            Json(serde_json::json!({"ok": true, "goal": goal}))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "embodiment")]
async fn embodiment_sensors(State(kernel): State<KernelState>) -> Json<Value> {
    let sensors = kernel.embodiment.available_sensors();
    let sensor_names: Vec<&str> = sensors
        .iter()
        .map(|s| match s {
            crate::embodiment::SenseModality::Text => "text",
            crate::embodiment::SenseModality::Image => "image",
            crate::embodiment::SenseModality::Audio => "audio",
            crate::embodiment::SenseModality::Video => "video",
            crate::embodiment::SenseModality::Haptic => "haptic",
            crate::embodiment::SenseModality::Olfactory => "olfactory",
            crate::embodiment::SenseModality::Gustatory => "gustatory",
        })
        .collect();
    Json(serde_json::json!({"sensors": sensor_names}))
}

#[cfg(feature = "embodiment")]
async fn embodiment_apps(State(kernel): State<KernelState>) -> Json<Value> {
    let apps = kernel.embodiment.list_agent_apps();
    Json(serde_json::json!({"apps": apps}))
}

// ── URI (Unified Robotics Interface) ─────────────────────────

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriSenseRequest {
    sensor_id: String,
}

#[cfg(feature = "uri")]
async fn uri_sense(
    State(kernel): State<KernelState>,
    Json(req): Json<UriSenseRequest>,
) -> Json<Value> {
    let result = kernel.uri.sense(&req.sensor_id);
    Json(serde_json::json!({
        "output": result.output,
        "tokens_used": result.tokens_used,
        "driver": result.driver,
        "mode": result.mode,
    }))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriActuateRequest {
    actuator_id: String,
    command: Value,
}

#[cfg(feature = "uri")]
async fn uri_actuate(
    State(kernel): State<KernelState>,
    Json(req): Json<UriActuateRequest>,
) -> Json<Value> {
    let result = kernel.uri.actuate(&req.actuator_id, &req.command);
    Json(serde_json::json!({
        "output": result.output,
        "tokens_used": result.tokens_used,
        "driver": result.driver,
        "mode": result.mode,
    }))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriTransformRequest {
    command: String,
    args: Value,
}

#[cfg(feature = "uri")]
async fn uri_transform(
    State(kernel): State<KernelState>,
    Json(req): Json<UriTransformRequest>,
) -> Json<Value> {
    let result = kernel.uri.transform(&req.command, &req.args);
    Json(serde_json::json!({
        "output": result.output,
        "tokens_used": result.tokens_used,
        "driver": result.driver,
        "mode": result.mode,
    }))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriStatusRequest {
    device_id: String,
}

#[cfg(feature = "uri")]
async fn uri_status(
    State(kernel): State<KernelState>,
    Json(req): Json<UriStatusRequest>,
) -> Json<Value> {
    let result = kernel.uri.status(&req.device_id);
    Json(serde_json::json!({
        "output": result.output,
        "tokens_used": result.tokens_used,
        "driver": result.driver,
        "mode": result.mode,
    }))
}

#[cfg(feature = "uri")]
async fn uri_switch(
    State(kernel): State<KernelState>,
    Json(req): Json<UriSwitchRequest>,
) -> Json<Value> {
    match kernel.hot_swap_uri(&req.driver_name) {
        Ok(()) => Json(serde_json::json!({
            "ok": true,
            "active_driver": kernel.uri.active_name(),
            "capability": kernel.uri.active_capability(),
            "hardware_type": kernel.uri.active_hardware_type(),
        })),
        Err(e) => Json(serde_json::json!({"ok": false, "error": e})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriSwitchRequest {
    driver_name: String,
}

#[cfg(feature = "uri")]
async fn uri_metrics(State(kernel): State<KernelState>) -> Json<Value> {
    let metrics = kernel.uri.get_metrics();
    Json(serde_json::json!({
        "metrics": metrics,
        "active_driver": kernel.uri.active_name(),
    }))
}

#[cfg(feature = "uri")]
async fn uri_drivers(State(kernel): State<KernelState>) -> Json<Value> {
    let drivers = kernel.uri.list_drivers();
    Json(serde_json::json!({ "drivers": drivers, "count": drivers.len() }))
}

#[cfg(feature = "uri")]
async fn uri_driver(
    State(kernel): State<KernelState>,
    Path(name): Path<String>,
) -> Json<Value> {
    match kernel.uri.get_driver(&name) {
        Some(info) => Json(serde_json::json!(info)),
        None => Json(serde_json::json!({"error": format!("Hardware driver '{}' not found", name)})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriRegisterDriverRequest {
    name: String,
    capability_level: u32,
    #[serde(default = "uri_default_token_cost")]
    token_cost: u64,
    #[serde(default)]
    hardware_type: String,
    #[serde(default)]
    config: HashMap<String, String>,
}

#[cfg(feature = "uri")]
fn uri_default_token_cost() -> u64 { 1 }

#[cfg(feature = "uri")]
async fn uri_driver_register(
    State(kernel): State<KernelState>,
    Json(req): Json<UriRegisterDriverRequest>,
) -> Json<Value> {
    let hw_type = if req.hardware_type.is_empty() { "generic" } else { &req.hardware_type };
    match kernel.uri.register_driver(
        &req.name, req.capability_level, req.token_cost, hw_type, req.config,
    ) {
        Ok(()) => Json(serde_json::json!({"ok": true, "name": req.name})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct UriUnregisterDriverRequest {
    name: String,
}

#[cfg(feature = "uri")]
async fn uri_driver_unregister(
    State(kernel): State<KernelState>,
    Json(req): Json<UriUnregisterDriverRequest>,
) -> Json<Value> {
    match kernel.uri.unregister_driver(&req.name) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

// ── Controller (Hardware Control Loops) ──────────────────────

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct CreatePidRequest {
    joint: String,
    kp: Option<f64>,
    ki: Option<f64>,
    kd: Option<f64>,
    saturation_max: Option<f64>,
    saturation_min: Option<f64>,
    frequency_hz: Option<u64>,
}

#[cfg(feature = "uri")]
async fn controller_pid_create(
    State(kernel): State<KernelState>,
    Json(req): Json<CreatePidRequest>,
) -> Json<Value> {
    let gains = crate::controller::PIDGains {
        kp: req.kp.unwrap_or(1.0),
        ki: req.ki.unwrap_or(0.0),
        kd: req.kd.unwrap_or(0.0),
        saturation_max: req.saturation_max.unwrap_or(100.0),
        saturation_min: req.saturation_min.unwrap_or(-100.0),
    };
    let ctrl = kernel.controller.create_pid_controller(&req.joint, gains, req.frequency_hz.unwrap_or(100));
    Json(serde_json::json!({"controller": ctrl}))
}

#[cfg(feature = "uri")]
async fn controller_pid_list(State(kernel): State<KernelState>) -> Json<Value> {
    let controllers = kernel.controller.list_pid_controllers();
    Json(serde_json::json!({"controllers": controllers, "count": controllers.len()}))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct PidSetpointRequest {
    id: String,
    setpoint: f64,
}

#[cfg(feature = "uri")]
async fn controller_pid_setpoint(
    State(kernel): State<KernelState>,
    Json(req): Json<PidSetpointRequest>,
) -> Json<Value> {
    match kernel.controller.set_pid_setpoint(&req.id, req.setpoint) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct PidStepRequest {
    id: String,
    measurement: f64,
    dt: f64,
}

#[cfg(feature = "uri")]
async fn controller_pid_step(
    State(kernel): State<KernelState>,
    Json(req): Json<PidStepRequest>,
) -> Json<Value> {
    match kernel.controller.step_pid_controller(&req.id, req.measurement, req.dt) {
        Ok(output) => Json(serde_json::json!({"output": output})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct TrajectoryCreateRequest {
    joint_names: Vec<String>,
    waypoints: Vec<crate::controller::Waypoint>,
}

#[cfg(feature = "uri")]
async fn controller_trajectory_create(
    State(kernel): State<KernelState>,
    Json(req): Json<TrajectoryCreateRequest>,
) -> Json<Value> {
    let traj = kernel.controller.create_trajectory(req.joint_names, req.waypoints);
    Json(serde_json::json!({"trajectory": traj}))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct TrajectoryStartRequest {
    id: String,
}

#[cfg(feature = "uri")]
async fn controller_trajectory_start(
    State(kernel): State<KernelState>,
    Json(req): Json<TrajectoryStartRequest>,
) -> Json<Value> {
    match kernel.controller.start_trajectory(&req.id) {
        Ok(()) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct TrajectoryStepRequest {
    id: String,
    dt: f64,
}

#[cfg(feature = "uri")]
async fn controller_trajectory_step(
    State(kernel): State<KernelState>,
    Json(req): Json<TrajectoryStepRequest>,
) -> Json<Value> {
    match kernel.controller.step_trajectory(&req.id, req.dt) {
        Ok(Some(positions)) => Json(serde_json::json!({"positions": positions, "done": false})),
        Ok(None) => Json(serde_json::json!({"done": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
async fn controller_trajectory_list(State(kernel): State<KernelState>) -> Json<Value> {
    let trajectories = kernel.controller.list_trajectories();
    Json(serde_json::json!({"trajectories": trajectories, "count": trajectories.len()}))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct SafetyMonitorCreateRequest {
    device_id: String,
    position_min: Option<f64>,
    position_max: Option<f64>,
    velocity_max: Option<f64>,
    torque_max: Option<f64>,
    temperature_max: Option<f64>,
}

#[cfg(feature = "uri")]
async fn controller_safety_create(
    State(kernel): State<KernelState>,
    Json(req): Json<SafetyMonitorCreateRequest>,
) -> Json<Value> {
    let limits = crate::controller::JointLimits {
        joint_name: req.device_id.clone(),
        position_min: req.position_min.unwrap_or(-180.0),
        position_max: req.position_max.unwrap_or(180.0),
        velocity_max: req.velocity_max.unwrap_or(180.0),
        torque_max: req.torque_max.unwrap_or(100.0),
        temperature_max: req.temperature_max.unwrap_or(85.0),
    };
    let monitor = kernel.controller.create_safety_monitor(&req.device_id, limits);
    Json(serde_json::json!({"safety_monitor": monitor}))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct SafetyCheckRequest {
    device_id: String,
    position: f64,
    velocity: f64,
    torque: f64,
    temperature: f64,
}

#[cfg(feature = "uri")]
async fn controller_safety_check(
    State(kernel): State<KernelState>,
    Json(req): Json<SafetyCheckRequest>,
) -> Json<Value> {
    match kernel.controller.check_safety(&req.device_id, req.position, req.velocity, req.torque, req.temperature) {
        Ok(()) => Json(serde_json::json!({"safe": true})),
        Err(violations) => Json(serde_json::json!({"safe": false, "violations": violations})),
    }
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct SafetyEStopRequest {
    device_id: String,
    reason: Option<String>,
}

#[cfg(feature = "uri")]
async fn controller_safety_estop(
    State(kernel): State<KernelState>,
    Json(req): Json<SafetyEStopRequest>,
) -> Json<Value> {
    match kernel.controller.trigger_e_stop(&req.device_id, &req.reason.unwrap_or_else(|| "manual".to_string())) {
        Ok(()) => Json(serde_json::json!({"ok": true, "estop_active": true})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
async fn controller_safety_estop_clear(
    State(kernel): State<KernelState>,
    Json(req): Json<SafetyEStopRequest>,
) -> Json<Value> {
    match kernel.controller.clear_e_stop(&req.device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true, "estop_active": false})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(feature = "uri")]
async fn controller_safety_monitors(State(kernel): State<KernelState>) -> Json<Value> {
    let monitors = kernel.controller.list_safety_monitors();
    Json(serde_json::json!({"safety_monitors": monitors, "count": monitors.len()}))
}

#[cfg(feature = "uri")]
async fn controller_safety_events(
    State(kernel): State<KernelState>,
    Path(device_id): Path<String>,
) -> Json<Value> {
    let events = kernel.controller.list_safety_events(&device_id);
    Json(serde_json::json!({"events": events, "count": events.len()}))
}

#[cfg(feature = "uri")]
#[derive(Deserialize)]
struct TransformAddRequest {
    name: String,
    parent: Option<String>,
    translation: [f64; 3],
    rotation: [f64; 4],
}

#[cfg(feature = "uri")]
async fn controller_transform_add(
    State(kernel): State<KernelState>,
    Json(req): Json<TransformAddRequest>,
) -> Json<Value> {
    kernel.controller.add_transform_frame(
        &req.name,
        req.parent.as_deref(),
        req.translation,
        req.rotation,
    );
    Json(serde_json::json!({"ok": true, "frame": req.name}))
}

#[cfg(feature = "uri")]
async fn controller_transform_list(State(kernel): State<KernelState>) -> Json<Value> {
    let frames = kernel.controller.list_transform_frames();
    Json(serde_json::json!({"frames": frames, "count": frames.len()}))
}

#[cfg(feature = "uri")]
async fn controller_stats(State(kernel): State<KernelState>) -> Json<Value> {
    let stats = kernel.controller.get_stats();
    Json(serde_json::json!({"controller_stats": stats}))
}

// ── Security Audit Export ────────────────────────────────────

#[derive(Deserialize)]
struct AuditExportRequest {
    format: Option<String>,
    sink: Option<String>,
    since: Option<String>,
}

async fn security_audit_export(
    State(kernel): State<KernelState>,
    Json(req): Json<AuditExportRequest>,
) -> Json<Value> {
    let fmt = req.format.as_deref().unwrap_or("jsonl");
    let sink = req.sink.as_deref().unwrap_or("stdout");
    let since = req.since.as_deref();

    let exporter: Box<dyn crate::audit_export::AuditExporter> = match sink {
        "stdout" => Box::new(StdoutExporter),
        path => Box::new(JsonlExporter::new(path)),
    };

    match kernel.export_audit_log(fmt, sink, since, exporter).await {
        Ok(result) => Json(serde_json::json!({"ok": true, "export": result})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

// ── Workflow Orchestration ──────────────────────────────────

async fn workflow_submit(
    State(kernel): State<KernelState>,
    Json(definition): Json<WorkflowDefinition>,
) -> Json<Value> {
    match kernel.submit_workflow(definition).await {
        Ok(instance) => Json(serde_json::json!({"ok": true, "workflow": instance})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[derive(Deserialize)]
struct WorkflowExecuteRequest {
    workflow_id: String,
}

async fn workflow_execute(
    State(kernel): State<KernelState>,
    Json(req): Json<WorkflowExecuteRequest>,
) -> Json<Value> {
    match kernel.execute_workflow(&req.workflow_id).await {
        Ok(()) => Json(serde_json::json!({"ok": true, "workflow_id": req.workflow_id})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn workflow_list(
    State(kernel): State<KernelState>,
) -> Json<Value> {
    let workflows = kernel.list_workflows().await;
    Json(serde_json::json!({"workflows": workflows, "count": workflows.len()}))
}

async fn workflow_get(
    State(kernel): State<KernelState>,
    Path(id): Path<String>,
) -> Json<Value> {
    match kernel.get_workflow(&id).await {
        Some(wf) => Json(serde_json::json!({"workflow": wf})),
        None => Json(serde_json::json!({"error": "Workflow not found"})),
    }
}

// ── Grid ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GridJoinRequest {
    node_id: String,
    address: String,
}

async fn grid_join(
    State(kernel): State<KernelState>,
    Json(req): Json<GridJoinRequest>,
) -> Json<Value> {
    kernel.grid.add_peer(req.node_id.clone(), req.address);
    Json(serde_json::json!({"ok": true, "node_id": kernel.grid.node_id()}))
}

async fn grid_peers(State(kernel): State<KernelState>) -> Json<Value> {
    let peers = kernel.grid.list_peers();
    Json(serde_json::json!({"peers": peers, "count": peers.len()}))
}

async fn grid_stats(State(kernel): State<KernelState>) -> Json<Value> {
    let stats = kernel.grid.stats();
    Json(serde_json::json!({"stats": stats}))
}

#[derive(Deserialize)]
struct GridHeartbeatRequest {
    node_id: String,
}

async fn grid_heartbeat(
    State(kernel): State<KernelState>,
    Json(req): Json<GridHeartbeatRequest>,
) -> Json<Value> {
    kernel.grid.record_heartbeat(&req.node_id);
    Json(serde_json::json!({"ok": true, "local_node_id": kernel.grid.node_id()}))
}

#[derive(Deserialize)]
struct GridMigrateRequest {
    pid: String,
    target_node: String,
}

async fn grid_migrate(
    State(kernel): State<KernelState>,
    Json(req): Json<GridMigrateRequest>,
) -> Json<Value> {
    match kernel.migrate_process(&req.pid, &req.target_node).await {
        Ok(frozen) => Json(serde_json::json!({"ok": true, "frozen": frozen})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

async fn grid_receive(
    State(kernel): State<KernelState>,
    Json(incoming): Json<crate::grid::MigratingProcess>,
) -> Json<Value> {
    let ctx = kernel.receive_process(incoming).await;
    Json(serde_json::json!({"ok": true, "pid": ctx.pid}))
}

#[derive(Deserialize)]
struct GridCipcForwardRequest {
    from: String,
    to: String,
    payload: Value,
}

async fn grid_cipc_forward(
    State(kernel): State<KernelState>,
    Json(req): Json<GridCipcForwardRequest>,
) -> Json<Value> {
    match kernel.cipc.route(&req.from, &req.to, req.payload) {
        Ok(msg) => Json(serde_json::json!({"ok": true, "message_id": msg.id})),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}
