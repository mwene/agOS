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
    ├── agOSKernel (kernel.rs)
    │     ├── scheduler.rs  ────── TokenScheduler
    │     ├── memory.rs     ────── HierarchicalMemoryManager
    │     ├── cipc.rs       ────── CIPCBus
    │     ├── security.rs   ────── SecurityEngine + SMTSolver
    │     ├── umi.rs        ────── UnifiedModelInterface
    │     └── panic_log     ────── RwLock<Vec<PanicLog>>
    │
    └── create_router (api.rs)
          ├── /health
          ├── /kernel/*       (5 endpoints)
          ├── /memory/*       (7 endpoints)
          ├── /cipc/*         (2 endpoints)
          ├── /security/*     (2 endpoints)
          ├── /umi/*          (6 endpoints)
          └── /sandbox/*      (3 endpoints)
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
  ├─ 2. UnifiedModelInterface::new()
  │     └─ AtomicU32 set to DRIVER_LOCAL (0)
  │     └─ Static drivers already compiled: LOCAL_DRIVER, CLOUD_DRIVER
  │
  ├─ 3. agOSKernel::new(umi)
  │     ├─ TokenScheduler::new()        — empty process vec
  │     ├─ HierarchicalMemoryManager::new() — empty all stores
  │     ├─ CIPCBus::new()               — empty msg log + route map
  │     ├─ SecurityEngine::new()         — init_default_policies()
  │     │   └─ SMTSolver::new()
  │     │   └─ Adds 3 constraints: blocked_actions, no_escalation, no_financial_transfer
  │     ├─ umi (passed in)
  │     └─ panic_log: empty Vec
  │
  ├─ 4. create_router(kernel)
  │     └─ axum::Router with 25 routes
  │     └─ CORS layer (allow all origins/methods/headers)
  │
  ├─ 5. tokio::net::TcpListener::bind("127.0.0.1:7400")
  │
  └─ 6. axum::serve(listener, router)
        └─ HTTP server ready
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

### api.rs — REST Router

The router is built in `create_router(kernel)`:

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
| `/memory/*` | 7 | l1 write/read, l2 compress, vector store, page-in, stats, episodic |
| `/cipc/*` | 2 | route, history |
| `/security/*` | 2 | verify, audit |
| `/umi/*` | 6 | reason, embed, compress, switch, metrics |
| `/sandbox/*` | 3 | install, launch, apps |

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
    pub fn new(umi: UnifiedModelInterface) -> Self {
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
| `async-trait` | 0.1 | (Reserved for future async trait use) |

### Build Commands

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Check without building (fast)
cargo check

# Run lints
cargo clippy

# Format code
cargo fmt

# Run all tests
cargo test
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
cargo test
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
| **cIPC** | Cognitive Inter-Process Communication — structured message bus between agents |
| **L1** | Level 1 memory — active context window, per-process |
| **L2** | Level 2 memory — compressed graph cache, per-process |
| **PID** | Process ID — unique identifier for each agent instance |
| **Ring 0** | The innermost security ring — SMT-constrained policy enforcement |
| **SMT** | Satisfiability Modulo Theories — constraint satisfaction solver |
| **Syscall** | A UMI system call (REASON, EMBED, COMPRESS, SWITCH) |
| **UMI** | Unified Model Interface — driver abstraction layer for AI models |
| **System 1** | Priority ≤ 1 processes (reflexive, fast, lower resource share) |
| **System 2** | Priority ≥ 2 processes (deliberate, deep reasoning, higher resource share) |
| **.aapp** | Agentic Application package — declarative agent blueprint |
