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
    │     ├── embodiment.rs    ────── EmbodimentLayer (feature: embodiment)
    │     ├── orchestrator.rs  ────── SubAgentOrchestrator
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
          ├── /workflow/*       (2 endpoints)
          ├── /embodiment/*     (8 endpoints, feature: embodiment)
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
| `embodiment` | `HashMap<String, EmbodimentGoal>` + `HashMap<String, ActionPlan>` | `std::sync::Mutex` | std::sync::Mutex |
| `orchestrator` | `HashMap<String, WorkflowInstance>` | `std::sync::Mutex` | std::sync::Mutex |
| `audit_export` | Stateless (trait implementations) | — | — |
| `telemetry` | Global OTLP tracer (static) | `opentelemetry::global` | Lock-free |

---

## Module Dependency Graph

```
                    ┌─────────────┐
                    │   kernel    │
                    │ (agOSKernel)│
                    └──┬──┬──┬──┬─┘
          ┌────────────┘  │  │  └──────────────┐
          │               │  │                 │
          ▼               ▼  ▼                 ▼
    ┌─────────┐    ┌──────────┐    ┌────────────┐
    │scheduler│    │  memory  │    │  security  │
    └─────────┘    └──────────┘    └────────────┘
                          │
          ┌───────────────┼───────────────┐
          │               │               │
          ▼               ▼               ▼
    ┌─────────┐    ┌──────────┐    ┌────────────┐
    │  cipc   │    │   umi    │    │  sandbox   │
    └─────────┘    └──────────┘    └────────────┘
                         │
                         ▼
                   ┌──────────┐
                   │  grid    │  ← always compiled, no feature gate
                   └──────────┘
                   ┌──────────┐
                   │DriverState│
                   │ (statics) │
                   └──────────┘
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
  │     └─ panic_log: empty Vec
  │
  ├─ 4. create_router(kernel)
  │     └─ axum::Router with core routes
  │     └─ + POST /scheduler/policy, GET /scheduler/policy
  │     └─ + POST /workflow/run, GET /workflow/status/:id
  │     └─ + POST /security/audit/export
  │     └─ (with feature wasm-sandbox: + POST /sandbox/execute)
  │     └─ (with feature embodiment: + 8 /embodiment/* routes)
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

### grid.rs — CognitiveGrid

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

The Orchestrator manages **workflow DAGs** defined in `.aapp` packages. Each workflow consists of sequential and/or parallel steps, each of which spawns a sub-agent (another `.aapp` process).

#### Internal State

```rust
pub struct SubAgentOrchestrator {
    workflows: Mutex<HashMap<String, WorkflowInstance>>,
}
```

`WorkflowInstance` tracks the execution state:

```rust
pub struct WorkflowInstance {
    pub id: String,
    pub definition: WorkflowDefinition,
    pub status: WorkflowStatus,
    pub current_step: String,
    pub steps: HashMap<String, StepInstance>,
    pub created_at: String,
}

pub struct StepInstance {
    pub status: StepStatus,            // Pending, Running, Succeeded, Failed, TimedOut
    pub child_pid: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
```

#### Key Methods

| Method | Locking | Description |
|--------|---------|-------------|
| `create_workflow(def)` | Mutex write | Creates instance, validates DAG, sets initial step |
| `advance_workflow(id, step_id, result)` | Mutex write | Advances to next step based on on_success/on_failure |
| `get_status(id)` | Mutex read | Returns full instance details |
| `list_workflows()` | Mutex read | Returns all instances |
| `fail_workflow(id)` | Mutex write | Marks workflow as failed, cleans up children |

#### DSL Parsing

Workflows are parsed from `manifest.json` by `AAppLoader` in `sandbox.rs`. The DSL is a JSON array of steps, each supporting:
- `id` — step identifier
- `agent` — `.aapp` name to spawn
- `timeout` — seconds before forced failure
- `on_success` / `on_failure` — next step transitions
- `parallel` — list of `{agent}` for concurrent execution
- `join` — `"all"` or `"any"`

#### Integration with Kernel

The kernel orchestrator is called from workflow REST handlers:
1. `POST /workflow/run` — loads the `.aapp`, checks for a `workflow` field in the manifest, creates a `WorkflowInstance`, and spawns the first step.
2. `GET /workflow/status/:id` — reads current state from the orchestrator.

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
cargo build --features "z3-solver,wasm-sandbox,embodiment"

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
cargo test --features "z3-solver,wasm-sandbox,embodiment"
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

# All features (46 tests: 22 embodiment + 9 memory + 2 sandbox + 4 Z3 + 9 grid)
cargo test --features "z3-solver,wasm-sandbox,embodiment"
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

The integration test is the 28-step Java `DemoOrchestrator`. It exercises every REST endpoint and verifies responses:

```bash
# Terminal 1
cd agos-kernel && cargo run

# Terminal 2
cd agos-demo
curl -X POST http://127.0.0.1:8080/api/demo/run
curl http://127.0.0.1:8080/api/demo/status
```

All 28 steps must pass with no failures. The demo is the CI gate.

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
