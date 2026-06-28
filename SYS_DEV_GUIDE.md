# System Developer Guide

This guide covers the internal architecture, module interfaces, concurrency model, and extension patterns of the agOS kernel. It is intended for developers contributing to the kernel itself — adding subsystems, modifying core logic, or extending the REST API.

---

## Table of Contents

- [System Architecture](#system-architecture)
- [Module Dependency Graph](#module-dependency-graph)
- [Boot Sequence](#boot-sequence)
- [Module Deep Dives](#module-deep-dives)
  - [kernel.rs — agOSKernel Orchestrator](#kernelrs--agoskernel-orchestrator)
  - [scheduler.rs — TokenScheduler](#schedulerrs--tokenscheduler)
  - [memory.rs — HierarchicalMemoryManager](#memoryrs--hierarchicalmemorymanager)
  - [cipc.rs — CIPCBus](#cipcrs--cipcbus)
  - [security.rs — SecurityEngine + SMTSolver](#securityrs--securityengine--smtsolver)
  - [umi.rs — UnifiedModelInterface](#umirs--unifiedmodelinterface)
  - [sandbox.rs — AAppLoader](#sandboxrs--aapploader)
  - [z3_solver.rs — Z3PolicySolver](#z3_solversr--z3policysolver)
  - [sandbox_wasm.rs — WasmSandbox](#sandbox_wasmrs--wasmsandbox)
  - [embodiment.rs — EmbodimentLayer](#embodimentrs--embodimentlayer)
  - [uri.rs — UnifiedRoboticsInterface](#urirs--unifiedroboticsinterface)
  - [controller.rs — ControllerManager](#controllersr--controllermanager)
  - [grid.rs — CognitiveGrid](#gridrs--cognitifrid)
  - [orchestrator.rs — SubAgentOrchestrator](#orchestratorrs--subagentorchestrator)
  - [audit_export.rs — AuditExporter](#audit_exportrs--auditexporter)
  - [telemetry.rs — OpenTelemetry Init](#telemetryrs--opentelemetry-init)
  - [api.rs — REST Router](#apirs--rest-router)
- [Concurrency Model](#concurrency-model)
- [Adding a New Kernel Subsystem](#adding-a-new-kernel-subsystem)
- [Adding a New REST Endpoint](#adding-a-new-rest-endpoint)
- [The Build System](#the-build-system)
- [Error Handling Conventions](#error-handling-conventions)
- [Testing Strategy](#testing-strategy)
- [Performance Considerations](#performance-considerations)
- [Glossary](#glossary)

---

## System Architecture

```
  main.rs
    │
    ├── init_telemetry (telemetry.rs)
    │     └── OTLP tracer layer
    │
    ├── agOSKernel (kernel.rs)
    │     ├── scheduler.rs     ────── TokenScheduler
    │     ├── memory.rs        ────── HierarchicalMemoryManager
    │     ├── cipc.rs          ────── CIPCBus
    │     ├── security.rs      ────── SecurityEngine + SMTSolver (+ Z3PolicySolver with feature)
    │     ├── umi.rs           ────── UnifiedModelInterface
│     ├── grid.rs          ────── CognitiveGrid (always compiled)
│     ├── uri.rs           ────── UnifiedRoboticsInterface (feature: uri)
│     ├── controller.rs    ────── ControllerManager (feature: uri)
│     ├── embodiment.rs    ────── EmbodimentLayer (feature: embodiment)
    │     ├── orchestrator.rs  ────── SubAgentOrchestrator (loops, parallel, submit/execute API)
    │     ├── marketplace.rs   ────── AgentMarketplace (catalog, search, templates, build, recommend)
    │     ├── governance.rs    ────── ContractEngine (contracts, compliance, policies, audit)
    │     ├── audit_export.rs  ────── AuditExporter trait + impls
    │     └── panic_log        ────── RwLock<Vec<PanicLog>>
    │
    └── create_router (api.rs)
          ├── /health
          ├── /kernel/*         (5 endpoints)
          ├── /scheduler/*      (2 endpoints)
          ├── /memory/*         (8 endpoints)
          ├── /cipc/*           (2 endpoints)
          ├── /security/*       (3 endpoints, +1 export)
          ├── /umi/*            (6 endpoints)
          ├── /sandbox/*        (3+1 endpoints, +1 with wasm-sandbox)
          ├── /workflow/*       (4 endpoints: submit, execute, list, :id)
          ├── /marketplace/*    (7 endpoints: list, search, agent, scan, templates, build, recommend)
          ├── /governance/*     (8 endpoints: contract create/get/list/remove, compliance, audit, policy create/list)
      ├── /embodiment/*     (8 endpoints, feature: embodiment)
      ├── /uri/*            (10 endpoints, feature: uri)
      ├── /controller/*     (17 endpoints, feature: uri)
      └── /grid/*           (7 endpoints, always on)
```

Each kernel subsystem is an independent struct that is composed into `agOSKernel`. Subsystems do not call each other directly — all coordination happens through `kernel.rs` methods or through the REST handlers in `api.rs`.

### Module Boundaries

| Module | Internal State | Shared Via | Locking |
|--------|---------------|------------|---------|
| `scheduler` | `Vec<CognitiveProcessContext>` | `Arc<RwLock<>>` | tokio::sync::RwLock |
| `memory` | L1: DashMap, L2: DashMap<DashMap>, Vector: RwLock<Vec>, Episodic: RwLock<Vec> | Direct fields | dashmap + std::sync::RwLock |
| `cipc` | `Vec<Message>` + `HashMap<String, Vec<String>>` | `std::sync::RwLock` | std::sync::RwLock |
| `security` | `Vec<Constraint>` + `Vec<AuditEntry>` | `std::sync::RwLock` | std::sync::RwLock |
| `umi` | `AtomicU32` (driver index) | `AtomicU32` | Lock-free |
| `kernel` | `Vec<PanicLog>` | `std::sync::RwLock` | std::sync::RwLock |
| `sandbox` | Stateless (filesystem reads) | — | — |
| `z3_solver` | `Context + Solver (behind Mutex)` | — | std::sync::Mutex |
| `sandbox_wasm` | `wasmtime::Engine` | — | — (Engine is Send+Sync) |
| `grid` | HashMap<String, PeerInfo>, Vec<GridMessage> | std::sync::Mutex | std::sync::Mutex |
| `uri` | DashMap for drivers and devices | std::sync::RwLock | std::sync::RwLock |
| `controller` | HashMap for PID/trajectory/safety state | std::sync::Mutex | std::sync::Mutex |
| `embodiment` | `HashMap<String, EmbodimentGoal>` + `HashMap<String, ActionPlan>` | `std::sync::Mutex` | std::sync::Mutex |
| `orchestrator` | `HashMap<String, WorkflowInstance>` | `std::sync::Mutex` | std::sync::Mutex |
| `audit_export` | Stateless (trait implementations) | — | — |
| `telemetry` | Global OTLP tracer (static) | `opentelemetry::global` | Lock-free |

---

## Module Dependency Graph

```
                    ┌──────────────────┐
                    │     kernel       │
                    │  (agOSKernel)    │
                    └──┬──┬──┬──┬──┬──┬─┘
          ┌────────────┘  │  │  │  │  └──────────────┐
          │               │  │  │  │                 │
          ▼               ▼  ▼  ▼  ▼                 ▼
    ┌─────────┐    ┌──────────┐    ┌─────────────┐
    │scheduler│    │  memory  │    │  security   │
    └─────────┘    └──────────┘    └─────────────┘
                          │
          ┌───────────────┼───────────────┬───────────────┐
          │               │               │               │
          ▼               ▼               ▼               ▼
    ┌─────────┐    ┌──────────┐    ┌────────────┐  ┌──────────────┐
    │  cipc   │    │   umi    │    │  sandbox   │  │marketplace  │
    └─────────┘    └──────────┘    └────────────┘  └──────────────┘
                          │                               │
                          ▼                               ▼
                    ┌──────────┐              ┌──────────────┐
                    │  grid    │              │ governance   │
                    └──────────┘              └──────────────┘
                    ┌──────────┐
                    │DriverState│
                    │ (statics) │
                    └──────────┘
                    ┌──────────┐       ┌──────────────┐
                    │   uri    │       │  controller  │ ← feature: uri
                    │ (feature)│──────>│ (PID, traj,  │
                    │ drivers  │       │  safety, xfm)│
                    └──────────┘       └──────────────┘
```

- **api.rs** depends on `kernel` and `sandbox` directly.
- **kernel.rs** composes all subsystems and coordinates multi-step operations.
- No circular dependencies between subsystems.

---

## Boot Sequence

```
main()
  │
  ├─ 1. Initialize tracing subscriber (env-filter logging)
  │
  ├─ 1b. init_telemetry()
  │       └─ OTLP tracer pipeline (optional, no-op if OTLP endpoint unavailable)
  │       └─ tracing_opentelemetry::layer() layered onto the registry
  │       └─ Returns ShutdownGuard for graceful span flush
  │
  ├─ 2. UnifiedModelInterface::new()
  │     └─ AtomicU32 set to DRIVER_LOCAL (0)
  │     └─ Static drivers already compiled: LOCAL_DRIVER, CLOUD_DRIVER
  │
  ├─ 3. agOSKernel::new(umi, "127.0.0.1:7400")
  │     ├─ TokenScheduler::new()            — empty process vec
  │     ├─ HierarchicalMemoryManager::new() — empty all stores
  │     ├─ CIPCBus::new()                   — empty msg log + route map
  │     ├─ SecurityEngine::new()            — init_default_policies()
  │     │   └─ SMTSolver::new()
  │     │   └─ Adds 3 constraints: blocked_actions, no_escalation, no_financial_transfer
  │     │   └─ (with feature z3-solver: wraps Z3PolicySolver in Mutex for Stage 1)
  │     ├─ CognitiveGrid::new(listen_addr)  — sets node_id from address
  │     ├─ SubAgentOrchestrator::new()      — empty workflow map
  │     ├─ umi (passed in)
  │     ├─ (with feature embodiment: EmbodimentLayer::new())
  │     ├─ (with feature uri: UnifiedRoboticsInterface::new())
  │     └─ panic_log: empty Vec
  │
  ├─ 4. create_router(kernel)
  │     └─ axum::Router with core routes
  │     └─ + POST /scheduler/policy, GET /scheduler/policy
  │     └─ + POST /workflow/run, GET /workflow/status/:id
  │     └─ + POST /security/audit/export
  │     └─ (with feature wasm-sandbox: + POST /sandbox/execute)
  │     └─ (with feature embodiment: + 8 /embodiment/* routes)
  │     └─ (with feature uri: + 10 /uri/* + 17 /controller/* routes)
  │     └─ 7 /grid/* routes (always compiled)
  │     └─ CORS layer (allow all origins/methods/headers)
  │
  ├─ 5. tokio::net::TcpListener::bind("127.0.0.1:7400")
  │
  └─ 6. axum::serve(listener, router)
        └─ HTTP server ready
        └─ On SIGTERM/SIGINT: shutdown_guard flush spans
```

---

## Module Deep Dives

### kernel.rs — agOSKernel Orchestrator

The kernel struct is the top-level composition point. It owns all subsystems and exposes orchestration methods.

```rust
pub struct agOSKernel {
    pub scheduler: TokenScheduler,
    pub memory: HierarchicalMemoryManager,
    pub cipc: CIPCBus,
    pub security: SecurityEngine,
    pub umi: UnifiedModelInterface,
    pub grid: CognitiveGrid,
    #[cfg(feature = "uri")]
    pub uri: UnifiedRoboticsInterface,
    #[cfg(feature = "uri")]
    pub controller: ControllerManager,
    panic_log: RwLock<Vec<PanicLog>>,
}
```

#### Key Methods

| Method | What It Does | Subsystems Touched |
|--------|-------------|-------------------|
| `spawn_process(manifest, parent_pid)` | Creates PID, registers w/ scheduler, logs episodic event | scheduler, memory |
| `kill_process(pid)` | Saves state, flushes L1, unregisters | scheduler, memory |
| `kernel_panic(pid, reason)` | Unregisters process, flushes L1, broadcasts alert, logs panic | scheduler, memory |
| `broadcast_alert(pid, msg)` | Writes CRITICAL severity event to episodic ledger | memory |
| `hot_swap_umi(driver_name)` | Delegates to umi.hot_swap() | umi |
| `migrate_process(pid, target_node)` | Freezes via grid, kills local, logs migration | scheduler, memory, grid |
| `receive_process(incoming)` | Unfreezes via grid, registers on scheduler, logs receipt | scheduler, grid |

#### Panic Flow

```
kernel_panic(pid, reason)
  ├─ scheduler.get_process(pid)   → read tokens_used
  ├─ PanicLog { pid, reason, timestamp, tokens_used }
  ├─ panic_log.push(entry)
  ├─ memory.log_episodic("KERNEL_PANIC", ...)
  ├─ memory.flush_l1(pid)
  ├─ scheduler.unregister(pid)
  └─ broadcast_alert(pid, reason)
       └─ memory.log_episodic("ALERT_BROADCAST", CRITICAL)
```

#### Extension Point

To add a new kernel-level operation (e.g., `suspend_process`), add the method to `agOSKernel` and call the relevant subsystems. The REST handler in `api.rs` then calls this new method.

---

### scheduler.rs — TokenScheduler

#### Internal State

```rust
pub struct TokenScheduler {
    processes: Arc<RwLock<Vec<CognitiveProcessContext>>>,
}
```

`CognitiveProcessContext` represents a running agent instance:

```rust
pub struct CognitiveProcessContext {
    pub pid: String,
    pub manifest: Value,           // The .aapp manifest JSON
    pub token_quota: u64,          // max_tokens from limits
    pub tokens_used: u64,          // cumulative consumption
    pub priority: u32,             // 1 = System 1, 2+ = System 2
    pub active_l1_context: Vec<Value>,
    pub state_machine: Value,      // logic_graphs.json
    pub parent_pid: Option<String>,
    pub goal_metric: Option<f64>,
    pub created_at: String,
}
```

#### Scheduling Model

The scheduler implements a **priority-weighted token allocation** model:

- **System 1** processes (priority ≤ 1): weight 0.3 — lower resource share, for reflexive/fast tasks.
- **System 2** processes (priority > 1): weight 1.0 — higher resource share, for deep reasoning.

#### Key Methods

| Method | Async? | Description |
|--------|--------|-------------|
| `register(ctx)` | yes | Add process to the scheduler |
| `unregister(pid)` | yes | Remove process by PID |
| `get_process(pid)` | yes | Clone process state (read-only snapshot) |
| `get_process_mut(pid, f)` | yes | Mutate process state via closure |
| `allocate_inference(pid, complexity)` | yes | Deduct tokens based on complexity × priority weight |
| `list_processes()` | yes | Clone all process states |
| `stats()` | yes | Counts for System 1 vs System 2 |

#### Concurrency

Uses `tokio::sync::RwLock` — allows concurrent reads (many list/status calls) with exclusive writes (spawn/kill). The lock is held across `.await` points, so critical sections are bounded.

#### Extension Point: New Scheduling Policy

To add a new scheduling algorithm:
1. Add a policy enum or config to `TokenScheduler`.
2. Modify `allocate_inference()` to implement the new policy.
3. Expose the policy selection via a new REST endpoint or manifest field.

---

### memory.rs — HierarchicalMemoryManager

Three-tier memory architecture:

```
  L1 (Active Context)     L2 (Graph Cache)      Vector RAM
  ┌─────────────────┐    ┌────────────────┐    ┌──────────────┐
  │ per-PID buffer  │    │ per-PID nested │    │ flat vector  │
  │ DashMap<String, │    │ DashMap        │    │ store with   │
  │ Vec<Value>>     │───>│ of DashMaps    │    │ cosine query │
  │ fast writes     │    │ compressed     │    │ page_in()    │
  │ flushable       │    │ snapshots      │    │              │
  └─────────────────┘    └────────────────┘    └──────────────┘
                           ┌────────────────┐
                           │ EpisodicLedger │
                           │ append-only    │
                           │ event log      │
                           │ (PID, type,    │
                           │  payload, ts)  │
                           └────────────────┘
```

#### Internal State

```rust
pub struct HierarchicalMemoryManager {
    l1: dashmap::DashMap<String, Vec<Value>>,
    l2: dashmap::DashMap<String, dashmap::DashMap<String, Value>>,
    vector_ram: RwLock<Vec<VectorEntry>>,
    episodic_ledger: RwLock<Vec<EpisodicEntry>>,
    compression_factor: f64,
}
```

#### Key Methods

| Method | Locking | Complexity |
|--------|---------|------------|
| `write_l1(pid, data)` | dashmap shard lock | O(1) |
| `append_l1(pid, item)` | dashmap shard lock | O(1) amortized |
| `read_l1(pid)` | dashmap shard lock | O(n) in context size |
| `write_l2(pid, key, val)` | dashmap shard lock (nested) | O(1) |
| `compress_to_l2(pid)` | L1 read + L2 write | O(n) |
| `store_vector(vec, meta)` | std::sync::RwLock write | O(1) |
| `page_in(query, top_k)` | std::sync::RwLock read | O(m·d + m log m) |
| `flush_l1(pid)` | dashmap shard lock | O(1) |
| `log_episodic(...)` | std::sync::RwLock write | O(1) |
| `get_stats()` | all stores read | O(n) across stores |

#### Cosine Similarity

```rust
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64
```

Page-in sorts all vectors by cosine similarity to the query and returns the top-k. This is a **brute-force scan** — fine for the demo scale, but for production you would swap in HNSW or IVF indices behind the same `page_in()` signature.

#### Extending Memory

To add a new memory tier (e.g., persistent SQLite-backed store):
1. Add a new field to `HierarchicalMemoryManager`.
2. Implement read/write/query methods.
3. Add REST endpoints in `api.rs` routing to the new methods.
4. Include the new tier in `get_stats()`.

---

### cipc.rs — CIPCBus

#### Message Structure

```rust
pub struct Message {
    pub id: String,          // UUID v4
    pub from: String,        // sender PID
    pub to: String,          // recipient PID
    pub payload: Value,      // arbitrary JSON
    pub timestamp: String,   // RFC 3339
}
```

#### Schema Validation

Messages can be validated against a JSON schema describing required fields:

```rust
pub struct SchemaValidator;
impl SchemaValidator {
    pub fn validate(payload: &Value, schema: &Value) -> bool
}
```

The schema is a `{"required": ["field1", "field2"]}` object. Validation checks that every listed field exists in the payload.

#### Route Permissions

```rust
pub fn register_process(&self, pid: &str, allowed_routes: Vec<String>)
```

- Empty `allowed_routes` = unrestricted (allow all).
- Non-empty = only listed target PIDs are routable.
- Default when launched via `sandbox/launch`: empty (unrestricted).

#### Concurrency

Uses `std::sync::RwLock` — the lock is never held across `.await`, so `tokio::sync::RwLock` is unnecessary. Lock contention is negligible at demo scale.

#### Extension: Dynamic Schema Registry

To add schema-based message validation per process or per message type, extend `register_process` to accept a schema map, and check in `route()` before storing.

---

### security.rs — SecurityEngine + SMTSolver

#### Two-Stage Verification

Every action goes through two checks:

```
ring0_verify(action_vector, permissions)
  │
  ├─ STAGE 1: SMT Constraint Check
  │     SMTSolver::check(claim)
  │     ├─ blocked_actions: "network:write", "filesystem:write", "sandbox:escape"
  │     ├─ no_escalation: capability != "admin:escalate"
  │     └─ no_financial_transfer: amount <= 0
  │     └─ If any violated → DENIED
  │
  └─ STAGE 2: Permission Match
        match_permission(action, pattern)
        ├─ "*" or "*:*" → allow anything
        ├─ "domain:*" → allow all actions in domain
        ├─ "domain:action" → exact match
        └─ Pattern segments support wildcards: "financial:read" matches "financial:*"
        └─ If no pattern matches → DENIED
```

#### Constraint Operators

| Operator | Types | Semantics |
|----------|-------|-----------|
| `==` | str, f64, Value | Equality |
| `!=` | str, Value | Inequality |
| `<=` | f64 | Numeric less-or-equal |
| `>=` | f64 | Numeric greater-or-equal |
| `in` | str in array | Set membership |
| `not_in` | str in array | Set exclusion |

#### Audit Log

Every `ring0_verify` call appends an `AuditEntry` with:
- `action` — the full action vector
- `result` — `"ALLOWED"` or `"DENIED"`
- `reason` — which constraint or permission check failed
- `timestamp` — RFC 3339

The audit is an in-memory append-only log. For production, wire it to a persistent store (SQLite, PostgreSQL, etcd).

#### Adding Constraints

Call `smt_solver.add_constraint()` — either in `init_default_policies()` or dynamically at runtime:

```rust
engine.smt_solver.add_constraint(Constraint {
    field: "action".to_string(),
    operator: "not_in".to_string(),
    value: serde_json::json!(["dangerous:action"]),
    rule: "custom_block".to_string(),
});
```

---

### umi.rs — UnifiedModelInterface

See the [UMI Driver Guide](UMI_DRIVER_GUIDE.md) for driver development. This section covers the interface internals.

#### Atomic Dispatch

```rust
pub struct UnifiedModelInterface {
    active: AtomicU32,
}

fn driver(&self) -> &'static DriverState {
    match self.active.load(Ordering::Relaxed) {
        DRIVER_CLOUD => &CLOUD_DRIVER,
        _ => &LOCAL_DRIVER,
    }
}
```

- `Ordering::Relaxed` is sufficient because there is no cross-thread happens-before dependency. Each HTTP request reads the index once.

#### Static Driver Storage

Drivers are `static` items, not heap-allocated:

```rust
static LOCAL_DRIVER: DriverState = DriverState { ... };
static CLOUD_DRIVER: DriverState = DriverState { ... };
```

This means:
- Zero-cost dispatch (no vtable, no dynamic dispatch).
- Drivers are compiled in, not loadable at runtime.
- All `DriverState` fields must be `const`-compatible (no `String`, no `Vec`).

#### Metrics Collection

```rust
pub fn get_metrics(&self) -> Vec<DriverMetrics>
```

Returns a snapshot of every driver's: name, capability_level, calls, total_tokens. The `AtomicU64` counters are relaxed-read, meaning metrics may lag behind by a few nanoseconds under concurrent load — acceptable for monitoring purposes.

---

### sandbox.rs — AAppLoader

#### Loading Logic

```rust
impl AAppLoader {
    pub fn load_from_path(path: &Path) -> Result<AAppPackage, String>
}
```

Reads the `.aapp` directory structure:
- `manifest.json` — required, parsed to `Value`
- `logic_graphs.json` — optional, defaults to `Value::Null`
- `tools/*.json` — optional, keyed by filename sans extension
- `validation_tests/*.json` — optional, sorted by filename

The loader returns an `AAppPackage`:

```rust
pub struct AAppPackage {
    pub name: String,
    pub manifest: Value,
    pub logic_graphs: Value,
    pub tools: HashMap<String, Value>,
    pub validation_tests: Vec<Value>,
}
```

#### Permission Check During Install

In `api.rs::sandbox_install`, the kernel checks every permission in the manifest against Ring 0:

```rust
for perm in &permissions {
    let action_vector = json!({"action": perm, "capability": perm, "target": "installer"});
    security.ring0_verify(&action_vector, &["*:*"])?;
}
```

This means even a `*:*` permission can't bypass the default SMT constraints (filesystem:write, network:write, sandbox:escape, admin:escalate).

#### Extension: Binary .aapp Format

To support a compressed binary format (tar.gz or custom), add a new method `load_from_bytes(data: &[u8])` that decompresses and delegates to the directory parsing logic.

---

### embodiment.rs — EmbodimentLayer

The Embodiment Layer implements the VLA (Vision-Language-Action) pipeline — processing multimodal user goals through Perception → Understanding → Planning → Execution → Feedback stages.

```rust
pub struct EmbodimentLayer {
    goals: Mutex<HashMap<String, EmbodimentGoal>>,
    active_plans: Mutex<HashMap<String, ActionPlan>>,
}
```

#### Key Types

```rust
pub enum SenseModality { Text, Image, Audio, Video, Haptic, Olfactory, Gustatory }

pub struct EmbodimentGoal {
    pub id: String,
    pub original_statement: String,
    pub supporting_inputs: Vec<SensoryInput>,
    pub parsed_intent: String,
    pub clarification_rounds: u32,
    pub needs_clarification: bool,
    pub status: GoalStatus,
    pub created_at: String,
    pub updated_at: String,
}

pub struct ActionPlan {
    pub goal_id: String,
    pub tasks: Vec<PipelineTask>,
    pub requires_clarification: Vec<String>,
    pub suggested_apps: Vec<String>,
}
```

#### Pipeline Flow

```
submit_goal(statement, sensory_inputs)
  │
  ├── GoalStatus::Received
  │
  └── run_pipeline(goal_id)
        ├── Understanding (UMI SYS_REASON)
        ├── Planning (create_plan)
        ├── If empty tasks → NeedsClarification (up to 3 rounds)
        └── Executing
              └── execute_plan → spawn_process per task
```

#### Key Methods

| Method | Async? | Description |
|--------|--------|-------------|
| `submit_goal()` | no | Creates goal, logs episodic event |
| `run_pipeline()` | yes | Full P-U-P-E pipeline using UMI |
| `execute_plan()` | yes | Spawns sub-agents for each task |
| `provide_clarification()` | yes | Accepts clarification, re-plans |
| `process_sensory_input()` | no | Wraps raw data into typed SensoryInput |
| `available_sensors()` | no | Returns all 7 SenseModality variants |
| `list_agent_apps()` | no | Returns known .aapp package names |

#### Feature Gate

- Feature: `embodiment` (no external dependency, just the module).
- Field: `pub embodiment: EmbodimentLayer` on `agOSKernel` behind `#[cfg(feature = "embodiment")]`.
- Routes: 8 endpoints under `/embodiment/*` behind `#[cfg(feature = "embodiment")]`.
- No effect when feature is disabled — kernel builds and runs unchanged.

#### Testing

```bash
cargo test --features embodiment
# 22 embodiment tests + 9 memory tests = 31 total
```

---

### uri.rs — UnifiedRoboticsInterface

The URI (Unified Robotics Interface) provides a hardware abstraction layer for sensors, actuators, and data transform pipelines — analogous to how UMI abstracts AI model inference.

```rust
pub struct UnifiedRoboticsInterface {
    drivers: RwLock<HashMap<String, HardwareDriver>>,
    devices: RwLock<HashMap<String, DeviceState>>,
    transforms: RwLock<Vec<DataTransform>>,
}
```

#### Key Types

```rust
pub struct HardwareDriver {
    pub name: String,
    pub driver_type: DriverType,    // Builtin, Runtime, Simulated
    pub capabilities: Vec<String>,  // e.g. ["sensor:temperature", "actuator:servo"]
    pub status: DriverStatus,       // Online, Offline, Error
    pub config: Value,              // driver-specific configuration
}

pub struct DeviceState {
    pub device_id: String,
    pub driver: String,
    pub last_value: Value,
    pub last_read: String,
    pub unit: String,
}

pub struct DataTransform {
    pub name: String,
    pub pipeline: Vec<TransformStep>,
}

pub enum TransformStep {
    Normalize { min: f64, max: f64 },
    Scale { factor: f64, offset: f64 },
    Threshold { lower: f64, upper: f64 },
    MovingAverage { window: usize },
    Custom { name: String, params: Value },
}
```

#### Key Methods

| Method | Async? | Locking | Description |
|--------|--------|---------|-------------|
| `sense(device_id, driver_name)` | no | RwLock read | Reads the current value of a device via its driver |
| `actuate(device_id, value, driver_name)` | no | RwLock read + write | Writes a value to an actuator via its driver |
| `transform(name, data)` | no | RwLock read | Applies a named pipeline of data transforms |
| `status(device_id)` | no | RwLock read | Returns driver + device status info |
| `switch(driver_name)` | no | RwLock write | Changes the active backend driver for a device |
| `register_driver(...)` | no | RwLock write | Registers a new runtime hardware driver |
| `unregister_driver(name)` | no | RwLock write | Removes a runtime hardware driver |
| `get_metrics()` | no | RwLock read | Returns per-driver call/error counters |
| `list_drivers()` | no | RwLock read | Lists all registered hardware drivers |

#### Concurrency

Uses `std::sync::RwLock` for driver/device/transform maps. Lock duration is negligible (HashMap lookups) — no `.await` held across locks.

#### Feature Gate

- Feature: `uri` (no external dependencies).
- Field: `pub uri: UnifiedRoboticsInterface` on `agOSKernel` behind `#[cfg(feature = "uri")]`.
- Routes: 10 endpoints under `/uri/*`.
- Ships with 4 built-in drivers: `SimulatedTemperature`, `SimulatedServo`, `SimulatedDistance`, `SimulatedIMU`.

---

### controller.rs — ControllerManager

The Controller Manager provides real-time control primitives: PID controllers, trajectory planning, safety monitors, and coordinate transforms.

```rust
pub struct ControllerManager {
    pid_controllers: Mutex<HashMap<String, PIDController>>,
    trajectories: Mutex<HashMap<String, TrajectoryPlan>>,
    safety_monitors: Mutex<HashMap<String, SafetyMonitor>>,
    transforms: Mutex<HashMap<String, Transform3D>>,
    event_log: Mutex<Vec<SafetyEvent>>,
    stats: Mutex<ControllerStats>,
}
```

#### Key Types

```rust
pub struct PIDController {
    pub id: String,
    pub kp: f64, pub ki: f64, pub kd: f64,
    pub setpoint: f64,
    pub integral: f64,
    pub prev_error: f64,
    pub dt: f64,
    pub output_limits: (f64, f64),
    pub last_update: String,
}

pub struct TrajectoryPlan {
    pub id: String,
    pub waypoints: Vec<TrajectoryWaypoint>,
    pub current_index: usize,
    pub status: TrajectoryStatus,    // Idle, Running, Completed, Paused, Failed
    pub interpolation: InterpolationMode, // Linear, Cubic, Spline
    pub created_at: String,
}

pub struct SafetyMonitor {
    pub id: String,
    pub device_id: String,
    pub condition: SafetyCondition,  // ThresholdAbove, ThresholdBelow, RateOfChange, Window
    pub threshold: f64,
    pub window_secs: Option<f64>,
    pub triggered: bool,
    pub created_at: String,
}

pub struct SafetyEvent {
    pub id: String,
    pub device_id: String,
    pub event_type: String,          // ESTOP_TRIGGERED, LIMIT_EXCEEDED, MONITOR_TRIGGERED
    pub message: String,
    pub timestamp: String,
    pub cleared: bool,
}
```

#### Key Methods

| Method | Locking | Description |
|--------|---------|-------------|
| `create_pid(cfg)` | Mutex write | Creates a PID controller with given gains and limits |
| `list_pid()` | Mutex read | Returns all PID controllers |
| `set_pid_setpoint(id, sp)` | Mutex write | Updates the setpoint of a PID controller |
| `pid_step(id, measurement)` | Mutex write | Executes one control step: error → P+I+D → clamped output |
| `create_trajectory(plan)` | Mutex write | Creates a trajectory with waypoints and interpolation mode |
| `list_trajectories()` | Mutex read | Returns all trajectory plans |
| `start_trajectory(id)` | Mutex write | Resets to waypoint 0, marks Running |
| `trajectory_step(id)` | Mutex write | Advances to next waypoint, returns current target |
| `create_safety_monitor(cfg)` | Mutex write | Creates a safety condition monitor |
| `check_safety(device_id, value)` | Mutex read | Checks all monitors for a device; returns triggered list |
| `trigger_estop(device_id, reason)` | Mutex write | Logs an ESTOP event for the device |
| `clear_estop(device_id)` | Mutex write | Clears unresolved ESTOP events for the device |
| `list_monitors()` | Mutex read | Returns all safety monitors |
| `events_for_device(device_id)` | Mutex read | Returns safety events for a given device |
| `add_transform(name, xform)` | Mutex write | Adds a named 4×4 coordinate transform |
| `list_transforms()` | Mutex read | Returns all registered transforms |
| `stats()` | Mutex read | Returns controller statistics |

#### Safety Event Flow

```
trigger_estop(device_id, reason)
  ├─ Creates SafetyEvent { event_type: "ESTOP_TRIGGERED", cleared: false }
  ├─ Logs to event_log
  └─ Returns the event

clear_estop(device_id)
  ├─ Finds all unresolved ESTOP events for device
  ├─ Sets cleared: true
  └─ Logs "ESTOP_CLEARED" event
```

All safety endpoints require `#[cfg(feature = "uri")]` — no effect when feature is disabled.

#### Feature Gate

- Feature: `uri` (shares feature flag with URI).
- Field: `pub controller: ControllerManager` on `agOSKernel` behind `#[cfg(feature = "uri")]`.
- Routes: 17 endpoints under `/controller/*`.

#### Testing

```bash
cargo test --features uri
# PID tests: create, setpoint, step
# Trajectory tests: create, start, step
# Safety tests: create monitor, check, estop, clear, events
# Transform tests: add, list
```

---

The Grid module implements **decentralised multi-node agent orchestration** — peer discovery, heartbeat liveness tracking, and agent migration. Unlike optional subsystems (Z3, WASM, Embodiment), the grid is **always compiled** (no feature gate).

#### Internal State

```rust
pub struct CognitiveGrid {
    node_id: String,
    peers: Mutex<HashMap<String, PeerInfo>>,
    messages: Mutex<Vec<GridMessage>>,
}
```

`PeerInfo` tracks the health and address of each peer:
```rust
pub struct PeerInfo {
    pub node_id: String,
    pub address: String,
    pub status: PeerStatus,
    pub last_seen: String,   // RFC 3339 timestamp
}
pub enum PeerStatus { Online, Stale, Offline }
```

#### Key Types

| Type | Role |
|------|------|
| `MigratingProcess` | Serialized process payload — `{ pid, agent_app, state }` |
| `GridMessage` | Internal message log enum — `Join`, `Heartbeat`, `Migrate`, etc. |
| `GridStats` | `{ peer_count: usize, message_count: u64 }` |
| `ResolveResult` | `{ Local, Remote(node_id) }` — for PID ownership lookup |

#### Key Methods

| Method | Locking | Description |
|--------|---------|-------------|
| `new(listen_addr)` | — | Sets `node_id` from the listen address |
| `node_id()` | — | Returns the node's identifier |
| `add_peer(id, addr)` | Mutex write | Adds or updates peer entry |
| `list_peers()` | Mutex read | Returns all `PeerInfo` |
| `remove_peer(id)` | Mutex write | Drops peer by node ID |
| `record_heartbeat(id)` | Mutex write | Updates `last_seen` and sets status to `Online` |
| `stale_peers(threshold_secs)` | Mutex read | Returns peers with `last_seen` older than threshold |
| `freeze_process(pid, ctx, app)` | — | Creates `MigratingProcess` from local state |
| `receive_process(incoming)` | — | Deserializes a `MigratingProcess` into a `CognitiveProcessContext` |
| `resolve_pid(pid)` | Mutex read | Checks peers for PID ownership; returns Local or Remote |
| `stats()` | Mutex read | Returns `GridStats` |

#### Concurrency

Uses `std::sync::Mutex` for `peers` and `messages`. Lock duration is negligible (HashMap insert/lookup/filter) — no `.await` held across the lock. The `Mutex` could be replaced with a `RwLock` if concurrent reads become a bottleneck.

#### Integration with Kernel

The kernel owns the grid and exposes two high-level operations:
- `migrate_process(pid, target_node)` — calls `grid.freeze_process()`, logs `PROCESS_MIGRATED` to episodic memory, kills the local process
- `receive_process(incoming)` — calls `grid.receive_process()`, registers the PID on the scheduler, logs `PROCESS_RECEIVED`

The kernel constructor now requires a `listen_addr: &str` to initialize the grid:
```rust
pub fn new(umi: UnifiedModelInterface, listen_addr: &str) -> Self
```

#### Testing

```bash
cargo test -- grid
# 9 tests: add/list/remove peers, heartbeat, stale detection, freeze/receive, PID resolution, stats
```

---

### orchestrator.rs — SubAgentOrchestrator

The Orchestrator manages **workflow DAGs** with support for sequential steps, parallel execution, and **loop steps** (RepeatUntil / While). Workflows can be submitted via API (ad-hoc definitions) or loaded from `.aapp` packages.

#### Internal State

```rust
pub struct SubAgentOrchestrator {
    workflows: RwLock<Vec<WorkflowInstance>>,
}
```

`WorkflowInstance` tracks the execution state:

```rust
pub struct WorkflowInstance {
    pub id: String,
    pub name: String,
    pub status: WorkflowStatus,         // Running, Completed, Failed(String), TimedOut
    pub current_step: Option<String>,
    pub step_results: HashMap<String, WorkflowStepResult>,
    pub created_at: String,
    pub updated_at: String,
    pub definition: Option<WorkflowDefinition>,  // stored for API-submitted workflows
}

pub struct WorkflowStepResult {
    pub step_id: String,
    pub agent: String,
    pub status: String,
    pub pid: Option<String>,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}
```

#### WorkflowStep (DSL)

```rust
pub struct WorkflowStep {
    pub id: String,
    pub agent: String,
    pub timeout_secs: Option<u64>,
    pub on_success: Option<String>,
    pub on_failure: Option<String>,
    pub parallel: Option<Vec<WorkflowStep>>,
    pub join: Option<String>,
    pub loop_type: Option<LoopType>,        // RepeatUntil or While
    pub loop_condition: Option<String>,      // evaluated by UMI
    pub max_iterations: Option<u32>,          // default 10
}

pub enum LoopType {
    RepeatUntil,
    While,
}
```

#### Key Methods

| Method | Locking | Description |
|--------|---------|-------------|
| `submit_workflow_definition(def)` | RwLock write | Creates instance from API definition, stores definition in instance |
| `execute_workflow(id, kernel)` | RwLock read | Finds instance, loads steps (from filesystem or stored definition), runs DAG |
| `execute_steps(name, steps, kernel)` | — | Iterates steps: loop steps go to `execute_loop_step`, parallel to tokio `join_all`, sequential to `spawn_process` |
| `execute_loop_step(workflow, step, kernel)` | — | Runs agent in a loop up to `max_iterations`, evaluates `loop_condition` via UMI between iterations |
| `check_condition(condition, kernel)` | — | Calls `umi.reason` to evaluate condition string, returns bool |
| `list_workflows()` | RwLock read | Returns all workflow instances |
| `get_workflow(id)` | RwLock read | Returns single instance by ID |

#### Loop Execution

Loop steps repeat the same agent until a condition is met. The orchestrator:

1. Spawns the agent as a sub-process (same manifest each iteration).
2. After each iteration, calls `umi.reason` with the condition string.
3. For `RepeatUntil`, continues if condition is false; for `While`, continues if condition is true.
4. Always stops after `max_iterations`, regardless of condition.

This enables patterns like "retry verification up to 3 times" or "audit while discrepancies remain".

#### Definition Fallback

When `execute_workflow` is called:
1. It first checks `aapps/{name}/logic_graphs.json` for a `steps` array (filesystem-based workflows from `.aapp` packages).
2. If no filesystem DSL exists, it falls back to the `definition` field stored on the `WorkflowInstance` (API-submitted workflows).
3. This dual path enables both `.aapp`-bundled workflows and ad-hoc API-submitted workflows.

#### Integration with Kernel

The kernel orchestrator is called from workflow REST handlers:
1. `POST /workflow/submit` — accepts a `WorkflowDefinition` JSON body, creates a `WorkflowInstance` and returns it.
2. `POST /workflow/execute` — accepts `{workflow_id}`, loads the definition, runs `execute_steps`.
3. `GET /workflow/list` — returns all instances via `list_workflows`.
4. `GET /workflow/{id}` — returns a single instance via `get_workflow`.

---

### marketplace.rs — AgentMarketplace

The Marketplace provides agent discovery, cataloging, template-based building, and goal→workflow recommendation. It uses `Mutex<HashMap>` for thread-safe access to the agent registry and templates.

#### Internal State

```rust
pub struct AgentMarketplace {
    registry: Mutex<HashMap<String, AgentListing>>,
    templates: Mutex<HashMap<String, AgentTemplate>>,
}
```

**AgentListing** stores metadata for each discovered agent:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Agent name from manifest |
| `version` | string | Semantic version |
| `description` | string | Purpose of the agent |
| `capabilities` | string[] | Capability tags from manifest |
| `permissions` | string[] | Required permissions |
| `token_quota` | u64 | Max tokens per run |
| `priority` | u32 | Scheduling priority |
| `has_workflow` | bool | Whether the agent defines a workflow |
| `states` | usize | Number of state machine states |
| `tool_count` | usize | Number of tool schemas |
| `source` | string | Origin (e.g. "local") |
| `installed` | bool | Whether the agent is installed |

#### Key Methods

| Method | Thread Safety | Description |
|--------|--------------|-------------|
| `scan_local(aapps_path)` | Mutex write | Reads all `.aapp` directories, creates `AgentListing` from each manifest, populates registry |
| `list_agents()` | Mutex read | Returns all registered agents, sorted by name |
| `get_agent(name)` | Mutex read | Returns a single agent by name |
| `search_agents(query)` | Mutex read | Filters by text query, capability, priority range, workflow flag; limits to 20 results |
| `list_templates()` | Mutex read | Returns all built-in template infos |
| `build_agent(request, aapps_path, kernel)` | Mutex write (indirect) | Creates `.aapp` directory, writes manifest.json + logic_graphs.json, creates empty tools/ and validation_tests/ dirs, rescans local, logs to episodic ledger |
| `recommend_agents(goal, kernel)` | Async, Mutex read | Lists available agents, constructs UMI prompt, parses response into `AgentRecommendation` with workflow steps |

#### Built-in Templates

Three templates are compiled into `init_templates()`: `data_analyzer` (2 states, 5000 tokens), `report_generator` (2 states, 8000 tokens), `validator` (2 states, 3000 tokens). Templates provide default permissions, token quotas, priority, and state machine definitions. Custom `states` in a build request override template defaults.

#### AgentListing.from_aapp_path

This function reads an `.aapp` directory and extracts metadata:
1. Loads `AAppPackage` via `AAppLoader::load_from_path`.
2. Extracts `description`, `capabilities`, `limits.max_tokens`, `priority`, `version`, `permissions` from manifest.
3. Counts states from `logic_graphs.json` (if present).
4. Populates `AgentListing` with sensible defaults for missing fields.

#### Goal→Workflow Recommendation

`recommend_agents` uses the UMI layer for reasoning:
1. Collects all available agents as a text catalog.
2. Builds a prompt with the user's goal and the agent catalog.
3. Calls `umi.reason` with the structured prompt.
4. Parses the `|`-delimited output into `RecommendedStep` entries.
5. Chains steps via `on_success` and returns a complete `WorkflowDefinition`.

**Locking note:** `Mutex` is sufficient because marketplace operations are fast (registry operations are hash lookups, file I/O is done outside the lock). The async `recommend_agents` only acquires the lock to read the registry, not during the UMI call.

#### REST Endpoints

| Method | Path | Handler |
|--------|------|---------|
| `GET` | `/marketplace/list` | `marketplace_list` |
| `POST` | `/marketplace/search` | `marketplace_search` |
| `GET` | `/marketplace/agent/{name}` | `marketplace_get_agent` |
| `POST` | `/marketplace/scan` | `marketplace_scan` |
| `GET` | `/marketplace/templates` | `marketplace_templates` |
| `POST` | `/marketplace/build` | `marketplace_build` |
| `POST` | `/marketplace/recommend` | `marketplace_recommend` |

---

### governance.rs — ContractEngine

The Governance subsystem provides compliance checking through **governance contracts** — declarative rules, limits, and success criteria that are evaluated against agent instances. The `ContractEngine` manages contracts, policies, and an audit log.

#### Internal State

```rust
pub struct ContractEngine {
    contracts: Mutex<HashMap<String, GovernanceContract>>,
    policies: Mutex<HashMap<String, GovernancePolicy>>,
    audit_log: Mutex<Vec<ComplianceReport>>,
}
```

#### GovernanceContract

```rust
pub struct GovernanceContract {
    pub contract_id: String,           // UUID v4
    pub name: String,
    pub description: String,
    pub rules: Vec<ContractRule>,       // compliance rules
    pub limits: ContractLimits,         // resource/permission limits
    pub success_criteria: SuccessCriteria, // success conditions
    pub created_by: String,
}
```

**ContractRule:**

| Field | Type | Description |
|-------|------|-------------|
| `field` | string | Dot-separated manifest path (e.g. `limits.max_tokens`) |
| `operator` | string | `"eq"`, `"lt"`, `"gt"`, `"contains"` |
| `value` | Value | Expected or threshold value |

**ContractLimits:**

| Field | Type | Description |
|-------|------|-------------|
| `max_tokens_per_run` | u64? | Max total token consumption |
| `max_subagents` | u32? | Max child processes |
| `max_execution_seconds` | u64? | Max wall-clock time |
| `max_memory_pages` | u32? | Max Vector RAM pages |
| `allowed_permissions` | string[]? | Allowlist for permission scopes |

**SuccessCriteria:**

| Field | Type | Description |
|-------|------|-------------|
| `min_goal_metric` | f64? | Minimum goal_metric threshold |
| `required_output_fields` | string[]? | Required output fields |
| `min_steps_completed` | u32? | Minimum completed steps |
| `no_error_states` | bool? | Reject any error state |

#### Compliance Checking

`check_compliance(instance: &AAppInstance) -> ComplianceReport`:

1. **Rules evaluation**: For each rule, navigates the manifest via dot-separated path, extracts the value, and compares using the specified operator (`eq`, `lt`, `gt`, `contains`).
2. **Limits checking**: Compares manifest values against contract limits for tokens, subagents, and permissions.
3. **Report generation**: Returns `ComplianceReport` with `contract_id`, `contract_name`, `agent_name`, `compliant: bool`, `violations: Vec<String>`, and `timestamp`.

#### Key Methods

| Method | Thread Safety | Description |
|--------|--------------|-------------|
| `register_contract(contract)` | Mutex write | Adds contract to registry, rejects duplicates |
| `get_contract(id)` | Mutex read | Returns contract by ID |
| `list_contracts()` | Mutex read | Returns all contracts sorted by name |
| `remove_contract(id)` | Mutex write | Removes contract, returns success |
| `check_agent_compliance(instance)` | Mutex read/write | Runs compliance against all contracts, logs report |
| `get_audit_log(limit)` | Mutex read | Returns last N compliance reports |
| `get_compliance_summary()` | Mutex read | Aggregates checks, compliant/violation counts, top violations |
| `create_policy(name, desc, contract_ids)` | Mutex write | Creates a policy bundling multiple contracts |
| `list_policies()` | Mutex read | Returns all policies sorted by name |

#### ComplianceReport

```rust
pub struct ComplianceReport {
    pub contract_id: String,
    pub contract_name: String,
    pub agent_name: String,
    pub compliant: bool,
    pub violations: Vec<String>,
    pub timestamp: String,           // RFC 3339
}
```

#### ComplianceSummary

```rust
pub struct ComplianceSummary {
    pub total_contracts: usize,
    pub total_checks: usize,
    pub compliant_checks: usize,
    pub violation_checks: usize,
    pub top_violations: Vec<(String, usize)>,  // violation text → count, top 10
}
```

#### REST Endpoints

| Method | Path | Handler |
|--------|------|---------|
| `POST` | `/governance/contract/create` | `governance_contract_create` |
| `GET` | `/governance/contract/{id}` | `governance_contract_get` |
| `GET` | `/governance/contracts` | `governance_contracts` |
| `POST` | `/governance/contract/{id}/remove` | `governance_contract_remove` |
| `GET` | `/governance/compliance` | `governance_compliance_summary` |
| `GET` | `/governance/audit` | `governance_audit_log` |
| `POST` | `/governance/policy/create` | `governance_policy_create` |
| `GET` | `/governance/policies` | `governance_policies` |

---

### audit_export.rs — AuditExporter

Provides structured export of the security audit log to external sinks for compliance and SIEM integration.

#### Trait Definition

```rust
#[async_trait]
pub trait AuditExporter: Send + Sync {
    async fn export(&self, entries: &[AuditEntry]) -> Result<(), String>;
}
```

#### Implementations

| Exporter | Sink ID | Description |
|----------|---------|-------------|
| `StdoutExporter` | `"stdout"` | Prints JSONL to stdout |
| `JsonlFileExporter` | `"jsonl_file"` | Writes JSONL to a local file |
| `S3Exporter` | `"s3"` | Uploads JSONL to S3-compatible object store via `rusty_s3` |

#### Format Helpers

```rust
pub fn format_entries(entries: &[AuditEntry], format: &str) -> Result<String, String>
```

- `"jsonl"` — one JSON object per line, fields: `timestamp`, `action`, `result`, `reason`
- `"csv"` — header row + data rows: `timestamp,action,result,reason`

#### API Integration

`POST /security/audit/export` accepts `{format, since, sink}` and:
1. Filters in-memory audit entries by `since` (if provided).
2. Calls `format_entries()` to produce the output string.
3. Instantiates the exporter from the `sink` config and calls `export()`.

---

### telemetry.rs — OpenTelemetry Init

Provides distributed tracing via OpenTelemetry's OTLP protocol, layered on top of the `tracing` ecosystem.

#### Initialization

```rust
pub fn init_telemetry() -> Option<TelemetryGuard> {
    let endpoint = std::env::var("OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());
    // Creates OTLP exporter, installs simple span processor,
    // returns a guard that flushes on Drop
}
```

#### Integration with Tracing

The OTLP layer is registered as a `tracing_subscriber::registry::Layer`:

```rust
let tracer = opentelemetry_otlp::new_pipeline()
    .install_simple().ok()?;
let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
registry.with(telemetry)
```

#### `#[instrument]` Usage

Key handler and kernel methods are annotated with `#[instrument(skip(self, ...))]` to create spans that include function name, duration, and structured fields (PID, agent name, endpoint, etc.).

#### Configuration

| Env Variable | Default | Description |
|-------------|---------|-------------|
| `OTLP_ENDPOINT` | `http://localhost:4317` | OTLP gRPC endpoint |
| `AGOS_SERVICE_NAME` | `agos-kernel` | Service name for trace identification |

---

### api.rs — REST Router

```rust
pub fn create_router(kernel: Arc<agOSKernel>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health_check))
        .route("/kernel/spawn", post(kernel_spawn))
        // ... 23 more routes ...
        .layer(cors)
        .with_state(kernel)
}
```

#### Handler Pattern

Every handler follows this pattern:

```rust
async fn handler_name(
    State(kernel): State<KernelState>,  // Arc<agOSKernel>
    Json(req): Json<RequestType>,       // deserialized request body
) -> Json<Value> {                      // always returns JSON
    let result = kernel.some_method(...).await;
    Json(json!({ "field": result }))
}
```

- All handlers are async.
- All handlers return `Json<Value>` (never raw strings or status codes).
- Request deserialization is automatic via serde.
- Errors are returned as JSON, never as HTTP error status codes (for simplicity at demo stage).

#### Route Table

| Prefix | Count | Notes |
|--------|-------|-------|
| `/health` | 1 | GET only, no auth |
| `/kernel/*` | 5 | spawn, kill, list, get, panic |
| `/scheduler/*` | 2 | GET/POST policy |
| `/memory/*` | 8 | l1 write/read, l2 compress, vector store, page-in, stats, episodic, episodic/compress |
| `/cipc/*` | 2 | route, history |
| `/security/*` | 3 | verify, audit, audit/export |
| `/umi/*` | 6 | reason, embed, compress, switch, metrics |
| `/sandbox/*` | 3 + 1 | install, launch, apps (+ execute with wasm-sandbox feature) |
| `/workflow/*` | 2 | run, status/:id |
| `/embodiment/*` | 8 | goal, goals, status, plan, clarify, execute, sensors, apps (with embodiment feature) |
| `/uri/*` | 10 | sense, actuate, transform, status, switch, metrics, drivers (with uri feature) |
| `/controller/*` | 17 | PID, trajectory, safety, estop, transform, stats (with uri feature) |
| `/grid/*` | 7 | join, peers, stats, heartbeat, migrate, receive, cipc/forward (always compiled) |

---

## Concurrency Model

### Lock Hierarchy

```
Layer              Lock Type              Held Across .await?
─────────          ─────────              ─────────────────
REST handlers      tokio::sync::RwLock    YES (scheduler)
Subsystems         std::sync::RwLock      NO (memory, cipc, security, kernel)
UMI                AtomicU32 (lock-free)  N/A
DashMap            Internal shard locks   NO (internal)
```

### Rules

1. **Never hold an `std::sync::RwLock` across `.await`.** This will deadlock the tokio runtime because the lock guard is not `Send`.
2. **Prefer `tokio::sync::RwLock` when the critical section contains async calls** (e.g., scheduler methods that may need future extension).
3. **Use dashmap for high-concurrency read/write maps** (L1, L2) — it shards internally so concurrent accesses to different keys are lock-free.
4. **Keep atomic operations at `Ordering::Relaxed`** for counters and the UMI driver index. Load-link/store-conditional semantics are not needed.

### Thread Safety

- `agOSKernel` is `Send + Sync` because all fields implement these traits.
- `Arc<agOSKernel>` is shared across axum handler clones.
- Static drivers (`LOCAL_DRIVER`, `CLOUD_DRIVER`) use `AtomicU64` for counters, which are `Sync`.

---

## Adding a New Kernel Subsystem

This is the recipe for adding a completely new capability (for example, a `Registry` service for agent service discovery).

### Step 1: Create the Module

`src/registry.rs`:

```rust
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub name: String,
    pub pid: String,
    pub endpoint: String,
}

pub struct ServiceRegistry {
    services: DashMap<String, ServiceEntry>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        ServiceRegistry { services: DashMap::new() }
    }

    pub fn register(&self, name: String, entry: ServiceEntry) {
        self.services.insert(name, entry);
    }

    pub fn lookup(&self, name: &str) -> Option<ServiceEntry> {
        self.services.get(name).map(|e| e.clone())
    }

    pub fn list(&self) -> Vec<ServiceEntry> {
        self.services.iter().map(|e| e.clone()).collect()
    }

    pub fn unregister(&self, name: &str) {
        self.services.remove(name);
    }
}
```

### Step 2: Add to lib.rs

```rust
pub mod registry;
```

### Step 3: Add to agOSKernel

```rust
pub struct agOSKernel {
    // ... existing fields ...
    pub registry: ServiceRegistry,
}

impl agOSKernel {
    pub fn new(umi: UnifiedModelInterface, listen_addr: &str) -> Self {
        agOSKernel {
            // ... existing initializations ...
            registry: ServiceRegistry::new(),
        }
    }
}
```

### Step 4: Add REST Endpoints in api.rs

```rust
// In create_router():
.route("/registry/register", post(registry_register))
.route("/registry/lookup/{name}", get(registry_lookup))
.route("/registry/list", get(registry_list))

// Handlers:
async fn registry_register(
    State(kernel): State<KernelState>,
    Json(req): Json<RegistryEntry>,
) -> Json<Value> {
    let entry = ServiceEntry {
        name: req.name.clone(),
        pid: req.pid.clone(),
        endpoint: req.endpoint.clone(),
    };
    kernel.registry.register(req.name.clone(), entry);
    Json(json!({"ok": true}))
}

async fn registry_lookup(
    State(kernel): State<KernelState>,
    Path(name): Path<String>,
) -> Json<Value> {
    match kernel.registry.lookup(&name) {
        Some(entry) => Json(json!({"entry": entry})),
        None => Json(json!({"error": "not found"})),
    }
}

async fn registry_list(
    State(kernel): State<KernelState>,
) -> Json<Value> {
    let services = kernel.registry.list();
    Json(json!({"services": services, "count": services.len()}))
}
```

### Step 5: Add Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_lookup() {
        let reg = ServiceRegistry::new();
        reg.register("alice".into(), ServiceEntry {
            name: "alice".into(),
            pid: "proc-abc".into(),
            endpoint: "/api/alice".into(),
        });
        let entry = reg.lookup("alice").unwrap();
        assert_eq!(entry.pid, "proc-abc");
    }
}
```

---

## Adding a New REST Endpoint

### Minimal Recipe

1. **Define a request struct** (or reuse `Value` for unstructured endpoints):
   ```rust
   #[derive(Deserialize)]
   struct MyRequest {
       field1: String,
       field2: u64,
   }
   ```

2. **Write the handler function** in `api.rs`:
   ```rust
   async fn my_handler(
       State(kernel): State<KernelState>,
       Json(req): Json<MyRequest>,
   ) -> Json<Value> {
       let result = kernel.some_method(req.field1, req.field2).await;
       Json(json!({"result": result}))
   }
   ```

3. **Add the route** in `create_router()`:
   ```rust
   .route("/my/endpoint", post(my_handler))
   ```

4. **Test with curl**:
   ```bash
   curl -X POST http://127.0.0.1:7400/my/endpoint \
     -H 'Content-Type: application/json' \
     -d '{"field1": "hello", "field2": 42}'
   ```

### Returning Errors

Return errors as JSON with the same shape, never as HTTP error codes (4xx/5xx). The demo client always checks a `.path("ok")` or `.path("error")` field:

```rust
if condition {
    Json(json!({"ok": true, "data": value}))
} else {
    Json(json!({"ok": false, "error": "something went wrong"}))
}
```

---

## The Build System

### Cargo.toml Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `axum` | 0.7 | HTTP framework, routing, extractors |
| `tokio` | 1 (full) | Async runtime, `RwLock`, TCP listener |
| `serde` | 1 (derive) | Serialization for all structs |
| `serde_json` | 1 | JSON `Value` for dynamic payloads |
| `uuid` | 1 (v4) | Process PIDs, message IDs, event IDs |
| `parking_lot` | 0.12 | (Reserved for future use — faster mutexes) |
| `dashmap` | 6 | Concurrent hash maps for L1, L2 |
| `tower-http` | 0.6 (cors) | CORS middleware for cross-origin requests |
| `tracing` | 0.1 | Structured logging |
| `tracing-subscriber` | 0.3 (env-filter) | Log level configuration via `RUST_LOG` |
| `chrono` | 0.4 (serde) | Timestamps (RFC 3339) |
| `tower` | 0.5 | Service layer for axum |
| `async-trait` | 0.1 | Async trait support for `AuditExporter` and `SchedulePolicy` |
| `z3` | 0.7 (optional) | Z3 SMT solver integration |
| `wasmtime` | 45 (optional) | WebAssembly runtime for sandbox tool execution |
| `base64` | 0.22 | Base64 decode for WASM binary upload + PEM parsing for keys |
| `opentelemetry` | 0.27 | OpenTelemetry API for distributed tracing |
| `opentelemetry-otlp` | 0.27 | OTLP exporter for OpenTelemetry spans |
| `tracing-opentelemetry` | 0.28 | Bridge between `tracing` spans and OpenTelemetry |
| `ed25519-dalek` | 2 | ED25519 signature verification for .aapp signing |
| `sha2` | 0.10 | SHA-256 hashing for .aapp signing |
| `rusty_s3` | 0.5 | S3-compatible object store client for audit export |

### Build Commands

```bash
# Build (debug, default features)
cargo build

# Build with optional features
cargo build --features "z3-solver,wasm-sandbox,embodiment,uri"

# Build (release)
cargo build --release

# Check without building (fast)
cargo check

# Run lints
cargo clippy

# Format code
cargo fmt

# Run all tests (default features)
cargo test

# Run all tests with all features
cargo test --features "z3-solver,wasm-sandbox,embodiment,uri"
```

### Logging

Set `RUST_LOG` to control verbosity:

```bash
RUST_LOG=debug cargo run
RUST_LOG=agos_kernel=trace cargo run
```

Available levels: `error`, `warn`, `info`, `debug`, `trace`.

---

## Error Handling Conventions

### Return Type Pattern

Kernel methods use `Result<T, String>` for fallible operations:

```rust
pub fn some_operation(&self, arg: &str) -> Result<SuccessType, String>
```

- **Success**: `Ok(value)` — the caller processes the value.
- **Failure**: `Err(description)` — a human-readable error string. No `anyhow` or `thiserror` — errors are simple strings propagated to the REST response.

### REST Error Shape

All errors are returned as `Json<Value>` with the same shape:

```json
{"ok": false, "error": "Human-readable description"}
```

Success responses vary by endpoint but generally follow:

```json
{"ok": true, ...fields...}
```

### When to Panic

**Never panic in library code.** The only panic is in `main.rs::tokio::net::TcpListener::bind().unwrap()` — if the port is unavailable, there is no recovery. In all other code paths, return `Err(String)`.

---

## Testing Strategy

### Unit Tests (in each module)

Each subsystem has `#[cfg(test)] mod tests` covering:
- Core operations (register/kill, write/read, route/match)
- Edge cases (empty inputs, non-existent PIDs, quota exhaustion)
- Permission matching (wildcards, exact matches, rejections)

```bash
# Default features (9 memory tests + 9 grid tests = 18 tests)
cargo test

# With z3-solver (+4 = 22 tests)
cargo test --features z3-solver

# With wasm-sandbox (+2 = 20 tests)
cargo test --features wasm-sandbox

# With embodiment (+22 = 40 tests)
cargo test --features embodiment

# With uri (+10 = 28 tests)
cargo test --features uri

# All features (56 tests: 22 embodiment + 9 memory + 2 sandbox + 4 Z3 + 9 grid + 10 URI+controller)
cargo test --features "z3-solver,wasm-sandbox,embodiment,uri"
```

Example test structure (from the pattern in each module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }
}
```

### Integration Test (via Demo)

The integration test is the Java `DemoOrchestrator`. It exercises REST endpoints across 13 phases (P1–P10 + UMI hot-swap) and verifies responses:

| Phase | Steps | What It Covers |
|-------|-------|----------------|
| P1 | 3 | Kernel health, UMI driver registration, active driver |
| P2 | 2 | `.aapp` installation (Financial Auditor, Accountant) |
| P3 | 3 | Process spawn, scheduler verification |
| P4 | 5 | UMI reason, L1 write, L2 compress, vector embed, page-in |
| P5 | 3 | cIPC route, response, history verification |
| UMI | 3 | Hot-swap, cloud reasoning, restore |
| P6 | 4 | Ring 0 ALLOW/DENY, kernel panic, L1 flush |
| P7 | 2 | Episodic ledger, audit log |
| P8 | 3 | Memory stats, UMI metrics, process listing |
| P9 | 17 | Robotic agents (Waiter/Chef/Supervisor), URI sensors, PID, trajectory, safety, E-Stop |
| P10 | 4 | Workflow submit with loop step, execute, list, get |

```bash
# Terminal 1
cd agos-kernel && cargo run

# Terminal 2
cd agos-demo
curl -X POST http://127.0.0.1:8080/api/demo/run
curl http://127.0.0.1:8080/api/demo/status
```

All steps must pass with no failures. The demo is the CI gate.

---

## Performance Considerations

### Current Bottlenecks

1. **Vector RAM page-in** — O(n) brute-force scan over all vectors. At 10k+ vectors, this becomes noticeable. Solution: swap in HNSW/IVF indices behind the same `page_in()` API.
2. **Episodic ledger** — in-memory `Vec` that grows unbounded. Solution: cap at N entries or wire to a persistent store.
3. **No caching** — every REST call runs the full handler. Low priority at demo scale.

### Lock Contention

- `dashmap` is sharded (default 16 shards) — concurrent access to different keys is lock-free.
- `std::sync::RwLock` in memory (for `vector_ram` and `episodic_ledger`) is uncontended at demo scale. For production, consider `tokio::sync::RwLock` or a lock-free data structure.

### Memory

- L1 context per process is bounded by `max_output × prompt_vector.len()` per UMI call.
- L2 grows with each compression event.
- Episodic ledger grows with each system event (spawn, panic, cIPC, etc.).

---

## Glossary

| Term | Definition |
|------|------------|
| **Figure of Speech** | Heuristic detection of idioms/metaphors in goal statements (20+ patterns) |
| **Semantic Ambiguity** | Flagged when a goal phrase has multiple possible meanings; resolved via UMI or user clarification |
| **Grid** | Decentralised peer-to-peer network of agOS nodes — discovery, heartbeat, migration |
| **cIPC** | Cognitive Inter-Process Communication — structured message bus between agents |
| **Embodiment** | The VLA pipeline layer — multimodal goal processing, sensory input, sub-agent orchestration |
| **L1** | Level 1 memory — active context window, per-process |
| **L2** | Level 2 memory — compressed graph cache, per-process |
| **PID** | Process ID — unique identifier for each agent instance |
| **Ring 0** | The innermost security ring — SMT-constrained policy enforcement |
| **SMT** | Satisfiability Modulo Theories — constraint satisfaction solver |
| **Syscall** | A UMI system call (REASON, EMBED, COMPRESS, SWITCH) |
| **UMI** | Unified Model Interface — driver abstraction layer for AI models |
| **VLA** | Vision-Language-Action — pipeline bridging perception, reasoning, and physical action |
| **WASM** | WebAssembly — sandboxed execution environment for untrusted tool code |
| **System 1** | Priority ≤ 1 processes (reflexive, fast, lower resource share) |
| **System 2** | Priority ≥ 2 processes (deliberate, deep reasoning, higher resource share) |
| **.aapp** | Agentic Application package — declarative agent blueprint |

---

## Deployment

agOS can be deployed in three environments: **bare-metal**, **virtual machine**, and **Docker container**.

### Prerequisites (all environments)

- Linux x86_64 (recommended) or macOS arm64
- 512 MB RAM minimum, 2 GB recommended
- Network interface on port 7400 (configurable in `main.rs`)

### Bare-Metal

```bash
# 1. Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Build with all features
cd agos-kernel
cargo build --release --features "z3-solver,wasm-sandbox,embodiment"

# 3. Create data directories
mkdir -p /var/lib/agos/aapps /var/lib/agos/data

# 4. Run
RUST_LOG=info ./target/release/agos-kernel

# Optional: install as systemd service
cat > /etc/systemd/system/agos-kernel.service << 'EOF'
[Unit]
Description=agOS Kernel
After=network.target

[Service]
Type=simple
User=agos
Group=agos
WorkingDirectory=/var/lib/agos
ExecStart=/usr/local/bin/agos-kernel
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now agos-kernel
```

### Virtual Machine

1. Provision a Linux VM (Debian 12 Bookworm or Ubuntu 24.04 LTS recommended).
2. Follow the Bare-Metal steps above inside the VM.
3. **Port forwarding** (if using Vagrant):
   ```ruby
   # Vagrantfile
   config.vm.network "forwarded_port", guest: 7400, host: 7400
   ```
4. **Security group** (cloud VMs): open TCP port 7400 for the kernel API.

### Docker

```bash
# Build and run with docker-compose (includes demo)
docker compose up --build -d

# Or build and run the kernel standalone
docker build -t agos-kernel .
docker run -d --name agos-kernel -p 7400:7400 agos-kernel

# Verify
curl http://localhost:7400/health
```

The `Dockerfile` uses a multi-stage build (Rust builder → Debian slim runtime) and the `docker-compose.yml` includes:
- agOS Kernel on port 7400
- agOS Demo (Spring Boot) on port 8080
- Shared Docker network for inter-container communication
- Persistent volume for kernel data
- Health checks on both services

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Logging level (`trace`, `debug`, `info`, `warn`, `error`) |
| `OTLP_ENDPOINT` | `http://localhost:4317` | OpenTelemetry OTLP gRPC endpoint |
| `AGOS_SERVICE_NAME` | `agos-kernel` | Service name for OpenTelemetry traces |
| `AGOS_TRUSTED_KEY` | _unset_ | PEM public key for .aapp signature verification (unset = skip) |
| listen address | `127.0.0.1:7400` | Kernel HTTP API address (hard-coded in `main.rs`) |

Authentication and API keys are configured at runtime via `POST /security/auth` (see [API Reference](API_REFERENCE.md)).

**Note**: `AGOS_TRUSTED_KEY` controls .aapp signature verification. When set, every sandbox install checks `signature.sig` against this public key. When unset, verification is skipped — compatible with unsigned packages.
