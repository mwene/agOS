# Features Guide

This guide documents all 10 phases of the agOS development roadmap — **Phases 1–5 (completed)** and **Phases 6–10 (planned)** — plus additional enhancements beyond the core roadmap. Each entry includes the motivation, design overview, implementation steps, files to modify, testing strategy, and open questions. Use this as a blueprint when picking up the next phase of work.

---

## Table of Contents

- [Phase 1 — Deterministic Firmware Scaffolding](#phase-1--deterministic-firmware-scaffolding)
- [Phase 2 — Unified Model Interface](#phase-2--unified-model-interface)
- [Phase 3 — Hierarchical Cognitive Memory + cIPC Bus](#phase-3--hierarchical-cognitive-memory--cipc-bus)
- [Phase 4 — Ring 0 Neuro-Symbolic Guard Layer](#phase-4--ring-0-neuro-symbolic-guard-layer)
- [Phase 5 — Sandbox Installer + Integration Smoke Test](#phase-5--sandbox-installer--integration-smoke-test)
- [Phase 6 — Episodic Compression Daemon](#phase-6--episodic-compression-daemon)
- [Phase 7 — Native Z3 SMT Solver](#phase-7--native-z3-smt-solver)
- [Phase 8 — WebAssembly Sandbox](#phase-8--webassembly-sandbox)
- [Phase 9 — Embodiment Layer (VLA Driver Pipeline)](#phase-9--embodiment-layer-vla-driver-pipeline)
- [Phase 10 — Decentralized Cognitive Grids](#phase-10--decentralized-cognitive-grids)
- [Additional Enhancements](#additional-enhancements)
  - [Persistent Storage (SQLite)](#persistent-storage-sqlite)
  - [Rate Limiting and QoS](#rate-limiting-and-qos)
  - [Token Accounting with Real Models](#token-accounting-with-real-models)
  - [OpenTelemetry Instrumentation](#opentelemetry-instrumentation)
  - [Kubernetes Operator](#kubernetes-operator)
  - [Pluggable Scheduler Policies](#pluggable-scheduler-policies)
  - [Sub-Agent Orchestration DSL](#sub-agent-orchestration-dsl)
  - [Structured Audit Export](#structured-audit-export)
  - [Secure .aapp Signing](#secure-aapp-signing)
- [Feature Prioritization](#feature-prioritization)
- [Design Principles for New Features](#design-principles-for-new-features)

---

## Phase 1 — Deterministic Firmware Scaffolding (Completed)

### Motivation

The foundational layer that positions agOS between the silicon firmware (Linux kernel) and the agent runtime. Traditional OS primitives (processes, files, sockets) are insufficient for cognitive workloads — agents need scheduling based on inference cycles, not CPU time.

### What Was Built

The Rust kernel crate with the core execution substrate:

- **`TokenScheduler`** (`src/scheduler.rs`) — manages `CognitiveProcessContext` instances, priority-weighted token allocation (System 1 weight 0.3, System 2 weight 1.0), process lifecycle (register, unregister, allocate).
- **Process model** — agents are spawned with a manifest defining `priority`, `max_tokens`, `permissions`, and `goal_metric`. Each gets a UUID-based PID.
- **`agOSKernel` orchestrator** (`src/kernel.rs`) — top-level struct composing all subsystems, coordinating multi-step operations like `spawn_process`, `kill_process`, and `kernel_panic`.
- **axum REST API** (`src/api.rs`) — HTTP server on `127.0.0.1:7400` with 25 endpoints.
- **Boot sequence** — tracing subscriber → UMI init → kernel init → router build → TCP bind → serve.

### Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | `tokio::main` entry point, kernel bootstrap |
| `src/kernel.rs` | `agOSKernel` orchestrator, panic/alert system |
| `src/scheduler.rs` | `TokenScheduler`, `CognitiveProcessContext` |
| `src/lib.rs` | Module declarations |
| `Cargo.toml` | Dependency manifest |
| `aapps/` | `.aapp` package directories |

### Testing

```bash
cargo build
cargo test
```

Manual smoke test via curl:

```bash
curl http://127.0.0.1:7400/health
curl http://127.0.0.1:7400/kernel/processes
```

### Key Design Decisions

- `agOSKernel` holds all subsystems as public fields (not traits) for simplicity — no dynamic dispatch, no DI container.
- The scheduler uses `tokio::sync::RwLock` because allocation may become async in future.
- All other subsystems use `std::sync::RwLock` or lock-free atomics.

---

## Phase 2 — Unified Model Interface (Completed)

### Motivation

Agents must invoke AI models without being coupled to any specific provider, architecture, or parameter count. Just as UNIX abstracts hardware through device files (`/dev/*`), UMI abstracts model inference through four syscalls.

### What Was Built

- **`UnifiedModelInterface`** (`src/umi.rs`) — abstraction layer with `AtomicU32` driver dispatch.
- **Two built-in drivers**:
  - `LocalEngine-v0` (capability level 2) — lightweight deterministic stub.
  - `CloudGPT-HighReason` (capability level 5) — higher-capability deterministic stub.
- **Four syscalls**: `SYS_REASON` (text generation), `SYS_EMBED` (vector embedding), `SYS_COMPRESS` (context summarization), `SYS_SWITCH` (hot-swap active driver).
- **Lock-free hot-swap** — `hot_swap()` is a single `AtomicU32::store(Ordering::Relaxed)`.
- **Metrics** — per-driver call count and total token tracking via `AtomicU64` counters.
- **`DriverMetrics`** — exposed via `GET /umi/metrics`.

### Key Files

| File | Purpose |
|------|---------|
| `src/umi.rs` | UMI struct, DriverState, all 3 built-in drivers |
| `src/api.rs` | `POST /umi/reason`, `POST /umi/embed`, `POST /umi/compress`, `POST /umi/switch`, `GET /umi/metrics` |

### Testing

```bash
# Reason
curl -X POST http://127.0.0.1:7400/umi/reason \
  -H 'Content-Type: application/json' \
  -d '{"prompt_vector": ["hello", "world"]}'

# Embed
curl -X POST http://127.0.0.1:7400/umi/embed \
  -H 'Content-Type: application/json' \
  -d '{"data": ["test"]}'

# Hot-swap
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "CloudGPT-HighReason"}'

# Metrics
curl http://127.0.0.1:7400/umi/metrics
```

### Key Design Decisions

- Drivers are `static` items, not heap-allocated — zero-cost dispatch, no vtable.
- All `DriverState` fields must be `const`-compatible (no `String`, no `Vec`).
- Deterministic stubs by default — plug in real model APIs per the [UMI Driver Guide](UMI_DRIVER_GUIDE.md).

---

## Phase 3 — Hierarchical Cognitive Memory + cIPC Bus (Completed)

### Motivation

Agents need a memory hierarchy analogous to CPU cache: fast volatile context (L1), compressed semantic cache (L2), and long-term vector-addressable storage (Vector RAM). They also need structured inter-process communication — raw sockets and stdin/stdout pipes are inadequate for semantic message passing.

### What Was Built

**Memory (`src/memory.rs`):**

- **L1 (Active Context)** — `DashMap<String, Vec<Value>>`, per-PID write/read/append/flush.
- **L2 (Graph Cache)** — `DashMap<String, DashMap<String, Value>>`, keyed by PID then by compression step.
- **Vector RAM** — `RwLock<Vec<VectorEntry>>` with `cosine_similarity()` and `page_in(query, top_k)`.
- **Episodic Ledger** — `RwLock<Vec<EpisodicEntry>>`, append-only event log with PID, event type, payload, timestamp.
- **`compress_to_l2(pid)`** — hashes L1 contents, stores compressed snapshot in L2.

**cIPC (`src/cipc.rs`):**

- **`CIPCBus`** — `RwLock<Vec<Message>>` for history, `RwLock<HashMap<String, Vec<String>>>` for route permissions.
- **`Message`** — id (UUID), from, to, payload (arbitrary JSON), timestamp.
- **`SchemaValidator`** — checks required fields against a JSON schema.
- **`route(from, to, payload)`** — validates route permissions, appends to history.
- **`get_history(pid, limit)`** — filtered message log.

### Key Files

| File | Purpose |
|------|---------|
| `src/memory.rs` | `HierarchicalMemoryManager`, `VectorEntry`, `EpisodicEntry`, `MemoryStats`, `cosine_similarity()` |
| `src/cipc.rs` | `CIPCBus`, `Message`, `SchemaValidator` |
| `src/api.rs` | `POST /memory/l1/*`, `POST /memory/l2/*`, `POST /memory/vector`, `POST /memory/page`, `GET /memory/stats`, `GET /memory/episodic`, `POST /cipc/route`, `GET /cipc/history` |

### Testing

```bash
# L1 write / read
curl -X POST http://127.0.0.1:7400/memory/l1/write \
  -H 'Content-Type: application/json' \
  -d '{"pid": "proc-test", "data": [{"type": "test"}]}'
curl http://127.0.0.1:7400/memory/l1/proc-test

# Vector RAM
curl -X POST http://127.0.0.1:7400/memory/vector \
  -H 'Content-Type: application/json' \
  -d '{"vector": [0.1, 0.2, 0.3], "metadata": {"source": "test"}}'

# cIPC
curl -X POST http://127.0.0.1:7400/cipc/route \
  -H 'Content-Type: application/json' \
  -d '{"from": "proc-a", "to": "proc-b", "payload": {"type": "test"}}'
curl http://127.0.0.1:7400/cipc/history
```

### Key Design Decisions

- `dashmap` for L1/L2 — thread-safe concurrent maps with shard-level locking.
- Brute-force O(n) cosine scan for page-in — acceptable at demo scale, replace with HNSW/IVF for production.
- Episodic ledger is in-memory only — Phase 6 adds compression; persistent storage is a post-10 enhancement.

---

## Phase 4 — Ring 0 Neuro-Symbolic Guard Layer (Completed)

### Motivation

Probabilistic model outputs must be constrained by mathematically verifiable state boundaries. Ring 0 is agOS's innermost security ring — it enforces policies that no agent, regardless of permissions or capability, can bypass.

### What Was Built

- **`SecurityEngine`** (`src/security.rs`) — orchestrates constraint checking and audit logging.
- **`SMTSolver`** — evaluates actions against a set of `Constraint` objects with operators: `==`, `!=`, `<=`, `>=`, `in`, `not_in`.
- **Three default policies**:
  - `blocked_actions` — `network:write`, `filesystem:write`, `sandbox:escape` are always denied.
  - `no_escalation` — `capability != "admin:escalate"`.
  - `no_financial_transfer` — `amount <= 0`.
- **Two-stage verification** in `ring0_verify(action_vector, permissions)`:
  1. SMT constraint check — hard-blocked actions.
  2. Permission wildcard matching — `"financial:*"` matches `"financial:read"`.
- **Audit log** — every ALLOW/DENY is recorded with action, result, reason, and timestamp.

### Key Files

| File | Purpose |
|------|---------|
| `src/security.rs` | `SecurityEngine`, `SMTSolver`, `AuditEntry`, `Constraint`, `match_permission()` |
| `src/api.rs` | `POST /security/verify`, `GET /security/audit` |

### Testing

```bash
# Should be ALLOWED
curl -X POST http://127.0.0.1:7400/security/verify \
  -H 'Content-Type: application/json' \
  -d '{"action_vector": {"action": "financial:read", "capability": "financial:read", "target": "/x", "amount": 0}, "permissions": ["financial:*"]}'

# Should be DENIED (blocked action)
curl -X POST http://127.0.0.1:7400/security/verify \
  -H 'Content-Type: application/json' \
  -d '{"action_vector": {"action": "filesystem:write", "capability": "admin:escalate", "target": "/etc/passwd", "amount": 0}, "permissions": ["*:*"]}'

# Audit log
curl http://127.0.0.1:7400/security/audit
```

### Key Design Decisions

- The SMT solver is a simple constraint evaluator, not full Z3 — Phase 7 upgrades to a real theorem prover.
- Permission matching supports `"domain:*"` wildcards and `"*:*"` (allow-all), but even `*:*` can't bypass the hard-coded blocked actions.
- Audit is in-memory. Export to persistent storage is planned as an enhancement.

---

## Phase 5 — Sandbox Installer + Integration Smoke Test (Completed)

### Motivation

Agents must be packaged as deployable units and go through a Ring 0 permission check at install time. The end-to-end smoke test validates every kernel subsystem in a single 28-step orchestration.

### What Was Built

**Sandbox (`src/sandbox.rs`):**
- **`AAppLoader`** — reads `.aapp` directory structure (`manifest.json`, `logic_graphs.json`, `tools/*.json`, `validation_tests/*.json`).
- **Install flow** — loads package, checks every declared permission against Ring 0, logs episodic event.
- **Launch flow** — loads package, spawns as a cognitive process, registers on cIPC bus, logs launch event.

**Two example `.aapp` packages:**
- `financial_auditor/` — 5-state state machine (validate_balance → verify_transactions → flag_anomaly → escalate_to_auditor → generate_report), `reconcile_accounts` tool.
- `accountant/` — 2-state state machine (compute_tax → file_return), `compute_tax_liability` tool.

**Demo integration (`agos-demo/`):**
- Java Spring Boot application with:
  - `AgosKernelClient` — typed REST client for all 25 kernel endpoints.
  - `DemoOrchestrator` — 28-step orchestration across 8 phases.
  - `WebController` + Thymeleaf template — live-updating dashboard.
- **28/28 steps passing** — all subsystems verified end-to-end.

### Key Files

| File | Purpose |
|------|---------|
| `src/sandbox.rs` | `AAppLoader`, `AAppPackage`, `InstallResult` |
| `aapps/financial_auditor/` | Example agent: manifest, logic_graphs, tools, validation_tests |
| `aapps/accountant/` | Example agent: manifest, logic_graphs, tools, validation_tests |
| `../agos-demo/` | Java Spring Boot integration demo |
| `../README.md` | Architecture, API reference, quick start |

### Testing

```bash
# Start kernel
cd agos-kernel && cargo run

# Install and launch via REST
curl -X POST http://127.0.0.1:7400/sandbox/install \
  -H 'Content-Type: application/json' \
  -d '{"name": "financial_auditor", "aapps_path": "aapps"}'

curl -X POST http://127.0.0.1:7400/sandbox/launch \
  -H 'Content-Type: application/json' \
  -d '{"name": "financial_auditor"}'

# Full 28-step integration test
cd agos-demo && mvn spring-boot:run
# Open http://127.0.0.1:8080 → click "Run Full Demo"
```

### Key Design Decisions

- `.aapp` is a directory on disk, not a binary archive — simple to create and inspect. A compressed binary format can be added later.
- Permission check at install time, not launch time — fail fast.
- Demo orchestrator is in Java to show platform independence (Rust kernel, Java client).

---

## Phase 6 — Episodic Compression Daemon

### Motivation

The episodic ledger is an in-memory `Vec<EpisodicEntry>` that grows unboundedly with every system event (spawn, kill, cIPC, panic, etc.). Without a background eviction and compression strategy, the kernel will exhaust memory under sustained load.

### Design

A background tokio task runs on a configurable interval and performs:

1. **Age-based tiering**: entries older than a threshold are candidates for compression.
2. **Semantic compression**: groups entries by `pid + event_type`, summarizes them into a single L2 entry.
3. **Episodic pruning**: raw entries are evicted from the ledger after compression.

```
                    ┌──────────────────┐
                    │ Episodic Ledger  │
                    │ (raw entries)    │
                    └────────┬─────────┘
                             │ age > threshold
                             v
                    ┌──────────────────┐
                    │ Compression Task │
                    │ (tokio::spawn)   │
                    └────────┬─────────┘
                             │
                    ┌────────┴─────────┐
                    │                  │
                    ▼                  ▼
              ┌──────────┐     ┌──────────────┐
              │ L2 Cache │     │ Pruned from  │
              │ (summary)│     │ Episodic Log │
              └──────────┘     └──────────────┘
```

### Implementation Steps

1. **Add config to `agOSKernel`**:
   ```rust
   pub struct EpisodicConfig {
       pub compression_interval_secs: u64,
       pub max_entries_before_compress: usize,
       pub entry_ttl_secs: u64,
   }
   ```

2. **Add a background task in `main.rs`**:
   ```rust
   let kernel_clone = kernel.clone();
   tokio::spawn(async move {
       let mut interval = tokio::time::interval(Duration::from_secs(60));
       loop {
           interval.tick().await;
           kernel_clone.memory.compress_episodic().await;
       }
   });
   ```

3. **Add `compress_episodic()` to `HierarchicalMemoryManager`**:
   ```rust
   pub async fn compress_episodic(&self) -> usize {
       let mut ledger = self.episodic_ledger.write().unwrap();
       let cutoff = Utc::now() - chrono::Duration::seconds(entry_ttl_secs as i64);
       let (fresh, stale) = ledger.iter().cloned()
           .partition(|e| e.timestamp > cutoff.to_rfc3339());
       // Group stale by (pid, event_type), write summaries to L2
       *ledger = fresh;
       stale.len()
   }
   ```

4. **Expose stats**: add `compressed_entries` to `MemoryStats`.
5. **Add REST endpoint**: `POST /memory/episodic/compress` for manual trigger.

### Files to Modify

| File | Change |
|------|--------|
| `agos-kernel/src/memory.rs` | Add `compress_episodic()`, config struct, stats field |
| `agos-kernel/src/kernel.rs` | Add `EpisodicConfig` to `agOSKernel` |
| `agos-kernel/src/main.rs` | `tokio::spawn` the background daemon |
| `agos-kernel/src/api.rs` | Add `POST /memory/episodic/compress` route |

### Testing

- Unit: seed 100+ entries, run compression, verify count dropped.
- Integration: start kernel, generate events, call manual compress endpoint, verify stats.

### Open Questions

- Should compression be configurable per-process?
- What summary format for L2? JSON blob, text summary via UMI SYS_COMPRESS, or both?

---

## Phase 7 — Native Z3 SMT Solver

### Motivation

The current `SMTSolver` evaluates constraints with a simple Rust evaluator — it supports `==`, `!=`, `<=`, `>=`, `in`, `not_in` but cannot reason about compound formulas, quantifiers, or optimization. Integrating [Z3](https://github.com/prove-rs/z3-rs) enables:

- **Symbolic constraint checking** — `(or (not blocked) (and allowed (<= amount 1000)))`
- **Optimization** — find the minimum token allocation that satisfies all constraints
- **Model finding** — generate a concrete action vector that passes all policies

### Design

```rust
pub struct Z3Solver {
    context: z3::Context,
    solver: z3::Solver,
}

impl Z3Solver {
    pub fn check(&self, claim: &Value, policies: &[Policy]) -> Result<(), Vec<String>> {
        // 1. Translate policies to Z3 assertions (Boolean + arithmetic constraints)
        // 2. Assert claim as a candidate model
        // 3. solver.check() → Sat/Unsat
        // 4. If Unsat, extract unsat core for error messages
    }
}
```

The `SecurityEngine` would keep a `Z3Solver` alongside (or replacing) the current `SMTSolver`.

### Implementation Steps

1. **Add Z3 dependency**:
   ```toml
   [dependencies]
   z3 = "0.7"   # or latest from https://crates.io/crates/z3
   ```

2. **Create `src/z3_solver.rs`**:
   ```rust
   use z3::{ast::*, *};

   pub struct Z3PolicySolver {
       ctx: Context,
   }

   impl Z3PolicySolver {
       pub fn new() -> Self {
           Z3PolicySolver { ctx: Context::new(&Config::new()) }
       }

       pub fn verify(&self, action: &Action, policies: &[Policy]) -> Result<(), String> {
           let solver = Solver::new(&self.ctx);
           for policy in policies {
               let constraint = self.policy_to_z3(&policy);
               solver.assert(&constraint);
           }
           let claim = self.action_to_z3(&action);
           solver.assert(&claim);
           match solver.check() {
               SatResult::Sat => Ok(()),
               SatResult::Unsat => {
                   let core = solver.get_unsat_core();
                   Err(format!("Blocked by: {:?}", core))
               }
               SatResult::Unknown => Err("Z3 timeout".into()),
           }
       }
   }
   ```

3. **Add `z3_solver` module to lib.rs**.
4. **Integrate into `SecurityEngine`** — call Z3 first, fall back to the existing evaluator, or replace it entirely.
5. **Wire to Ring 0 verify endpoint**.

### Constraints That Z3 Enables Today

| Current | Z3 | Benefit |
|---------|-----|---------|
| `not_in blocked_actions` | `(forall (a) (=> (blocked a) (not (= action a))))` | Quantified rules |
| Simple string matching | Regex/pattern matching on action strings | Wildcard `financial:*` becomes a formal regex |
| No optimization | `(minimize tokens)` | Find minimal-token action that satisfies policy |

### Files to Modify

| File | Change |
|------|--------|
| `agos-kernel/Cargo.toml` | Add `z3` dependency |
| `agos-kernel/src/lib.rs` | Add `pub mod z3_solver` |
| `agos-kernel/src/z3_solver.rs` | New file |
| `agos-kernel/src/security.rs` | Integration point in `ring0_verify` |

### Testing

- Unit: encode current 3 default policies in Z3, verify same ALLOW/DENY results.
- Fuzz: generate random action vectors, compare Z3 result vs evaluator result (should match).

### Open Questions

- Z3 adds ~30MB to the binary. Is the tradeoff worth it before Phase 10 scale?
- Should Z3 be an optional feature flag (`z3 = []` in Cargo.toml)?
- How to handle Z3 `Unknown` results — fail open or fail closed?

---

## Phase 8 — WebAssembly Sandbox

### Motivation

Tool execution currently lives outside the kernel — the `.aapp` declares tool schemas, but the actual execution requires an external runtime. Embedding a WebAssembly (WASM) sandbox allows agents to run **untrusted tool code** inside the kernel with hardware-enforced isolation.

### Design

```bash
.aapp/tools/reconcile.wasm  # compiled WAT → wasm32-unknown-unknown
```

The sandbox loader (`sandbox.rs`) loads `.wasm` files alongside `.json` tool schemas. The kernel invokes the WASM function via `wasmtime` with:

- **Memory limit**: configurable per-app (e.g., 1MB).
- **Fuel metering**: instructions executed = token consumption.
- **WASI preview 2**: limited I/O (stdout only, no filesystem/network by default).
- **Host functions**: the WASM module can call back into agOS (e.g., `c_ipc_send`, `memory_read_l1`, `umi_reason`) — all mediated by Ring 0 security.

```
  Agent Process
       │
       │ sandbox/launch
       ▼
  ┌──────────────────┐
  │   wasmtime::Store │
  │  ┌──────────────┐ │
  │  │ .wasm module │ │  ← sandboxed: fuel, memory, WASI
  │  └──────────────┘ │
  │  ┌──────────────┐ │
  │  │ Host Fn:     │ │  ← Ring 0 verified before execution
  │  │ cipc_send()  │ │
  │  │ memory_read()│ │
  │  │ umi_reason() │ │
  │  └──────────────┘ │
  └──────────────────┘
```

### Implementation Steps

1. **Add `wasmtime` dependency**:
   ```toml
   wasmtime = "24"  # or latest
   wasmtime-wasi = "24"
   ```

2. **Create `src/sandbox_wasm.rs`**:
   ```rust
   use wasmtime::*;

   pub struct WasmSandbox {
       engine: Engine,
   }

   impl WasmSandbox {
       pub fn new(fuel_per_call: u64) -> Self { ... }

       pub fn execute_tool(
           &self,
           wasm_bytes: &[u8],
           input: &Value,
           kernel: &agOSKernel,
           pid: &str,
       ) -> Result<Value, String> {
           let mut store = Store::new(&self.engine, ());
           store.set_fuel(fuel_per_call)?;
           let module = Module::new(&self.engine, wasm_bytes)?;
           // Link host functions with Ring 0 checks
           let linker = self.build_linker(kernel, pid)?;
           let instance = linker.instantiate(&mut store, &module)?;
           let run = instance.get_typed_func::<(i32, i32), i32>(&mut store, "run")?;
           // Marshal input to WASM linear memory, call run(), unmarshal output
           Ok(serde_json::json!({ "result": "..." }))
       }
   }
   ```

3. **Modify `AAppLoader`** to load `.wasm` files from `tools/` (alongside `.json`).
4. **Add fuel consumption to scheduler** — every WASM instruction burns tokens.
5. **Implement host functions** — `c_ipc_send`, `memory_read`, `umi_reason` wrappers that call `ring0_verify` first.
6. **Add REST endpoint** — `POST /sandbox/execute` that runs a tool by name with given input.

### Files to Modify

| File | Change |
|------|--------|
| `agos-kernel/Cargo.toml` | Add `wasmtime`, `wasmtime-wasi` |
| `agos-kernel/src/lib.rs` | Add `pub mod sandbox_wasm` |
| `agos-kernel/src/sandbox_wasm.rs` | New file |
| `agos-kernel/src/sandbox.rs` | WASM loading in `AAppLoader` |
| `agos-kernel/src/scheduler.rs` | Fuel-based token deduction |
| `agos-kernel/src/kernel.rs` | Expose host function helpers |
| `agos-kernel/src/api.rs` | Add `/sandbox/execute` |

### Testing

- Compile a minimal WAT module that adds two numbers, deploy as `.aapp`, execute via REST, verify result.
- Test fuel exhaustion: infinite loop WAT should be killed after quota.
- Test Ring 0: WASM calling a host function with blocked action (e.g., `filesystem:write`) should be denied.

### Open Questions

- WASM compile times — should modules be cached after first load?
- How to handle WASM `memory.grow` — respect `max_subagents` as a heuristic?
- Should WASM modules be sandboxed per-process or per-tool-call?

---

## Phase 9 — Embodiment Layer (VLA Driver Pipeline)

### Motivation

Agents should be able to control physical or simulated robots. The Embodiment Layer adds a **Vision-Language-Action (VLA) driver pipeline** that translates high-level agent intents (`"move arm to position X"`) into low-level motor commands via UMI-style drivers.

### Design

```
  Agent
   │  "pick up the cup"
   ▼
  ┌──────────────────────┐
  │  EmbodimentOrchestrator │
  │  ┌────────────────┐   │
  │  │ VLA Driver     │   │  ← Router to specific robot backend
  │  │ (UMI-like)     │   │
  │  └────────────────┘   │
  │  ┌────────────────┐   │
  │  │ Kinematics      │   │  ← Inverse kinematics, collision detection
  │  │ Simulator       │   │
  │  └────────────────┘   │
  │  ┌────────────────┐   │
  │  │ Sensor Pipeline │   │  ← Camera, LiDAR, proprioception
  │  └────────────────┘   │
  └──────────────────────┘
```

The layer introduces a new UMI syscall: `SYS_ACT`. While `SYS_REASON` returns text, `SYS_ACT` returns **action primitives** (joint angles, gripper position, velocity commands).

### Implementation Steps

1. **Create `src/embodiment.rs`**:
   ```rust
   pub struct EmbodimentOrchestrator {
       active_driver: AtomicU32,
   }

   pub enum ActionPrimitive {
       MoveJoints { angles: Vec<f64>, speed: f64 },
       MoveLinear { x: f64, y: f64, z: f64 },
       Grip { force: f64 },
       Wait { duration_secs: f64 },
   }

   impl EmbodimentOrchestrator {
       pub async fn act(&self, intent: &str, context: &Value) -> Result<Vec<ActionPrimitive>, String>;
       pub fn hot_swap_driver(&self, name: &str) -> Result<(), String>;
   }
   ```

2. **Add embodiment drivers**:
   - `SimulatedArm` — returns predefined action primitives.
   - `ROS2Driver` — publishes to ROS2 topics via `ros2-client` crate.
   - `MoonshotVLA` — calls a remote VLA model API.

3. **Register SYS_ACT as a UMI syscall** — extend `UnifiedModelInterface` with `act()` or create a parallel `EmbodimentInterface`.
4. **Add REST endpoints**: `POST /embodiment/act`, `POST /embodiment/switch`.

### Files to Modify

| File | Change |
|------|--------|
| `agos-kernel/src/embodiment.rs` | New file |
| `agos-kernel/src/lib.rs` | Add `pub mod embodiment` |
| `agos-kernel/src/kernel.rs` | Add `EmbodimentOrchestrator` field |
| `agos-kernel/src/api.rs` | Add `/embodiment/*` routes |

### Testing

- Unit: simulate a pick-and-place sequence, verify action primitives are well-formed.
- Integration (with simulator): spawn an agent, send `"move to home"` intent, verify joint angles match expected.

### Open Questions

- Should `SYS_ACT` be a UMI syscall or a separate interface? (UMI = model inference, Embodiment = physical action — they may share a pipeline but differ in output type.)
- How does the agent specify which robot? Via `manifest.json` or a runtime parameter?
- Safety: how to enforce hardware limits (joint stops, force limits) in the pipeline?

---

## Phase 10 — Decentralized Cognitive Grids

### Motivation

Single-node agOS is limited by available compute. Decentralized Cognitive Grids enable **multi-node agent orchestration** — agents on one node can cIPC to agents on another node, migrate between nodes, and coordinate globally.

### Design

```
  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
  │  agOS Node A │     │  agOS Node B │     │  agOS Node C │
  │  :7400       │←───→│  :7400       │←───→│  :7400       │
  └──────────────┘     └──────────────┘     └──────────────┘
         │                                       │
         │          Gossip Protocol               │
         └────────────────────────────────────────┘
                          │
                    ┌─────┴─────┐
                    │ Grid DNS  │  ← Service discovery, node registry
                    │ (etcd)    │
                    └───────────┘
```

### Implementation Steps

1. **Grid Discovery**:
   - Nodes register with a shared etcd/Consul cluster on startup.
   - Each node maintains a `HashMap<NodeId, SocketAddr>` of peers.
   - Heartbeat every 5s; nodes are evicted after 3 missed heartbeats.

2. **Remote cIPC**:
   - Extend `CIPCBus::route()` to check if `to` PID is local or remote.
   - For remote PIDs, forward the message via HTTP to the peer node's `/cipc/route` endpoint.
   - Add a `node_id` field to `Message`.

3. **Agent Migration**:
   - `POST /kernel/migrate { pid, target_node }` freezes the process state, serializes it, sends it to the target node's `POST /kernel/receive`, and unregisters locally.
   - The target node deserializes and registers the process.

4. **Global Episodic Ledger**:
   - Episodic events carry a `node_id`.
   - The ledger can be queried globally (`GET /grid/episodic?node=all`).
   - Each node serves as a ledger replica via gossip.

### Files to Modify

| File | Change |
|------|--------|
| `agos-kernel/src/grid.rs` | New file — discovery, gossip, remote routing |
| `agos-kernel/src/lib.rs` | Add `pub mod grid` |
| `agos-kernel/src/cipc.rs` | Remote routing logic |
| `agos-kernel/src/kernel.rs` | `migrate_process()`, `receive_process()` |
| `agos-kernel/src/api.rs` | `/grid/*`, `/kernel/migrate`, `/kernel/receive` |
| `agos-demo/...` | Grid dashboard showing cluster health |

### Testing

- Start 2 kernel instances on different ports (7400, 7401).
- Spawn an agent on node A, cIPC to an agent on node B.
- Migrate an agent from A to B, verify state is preserved.
- Kill node A, verify heartbeat eviction within 15s.

### Open Questions

- Conflict resolution: two nodes claim the same PID. Use `NodeId + PID` as the global identifier?
- Consistency model: eventual consistency for the episodic ledger? Strong consistency for process state?
- Network partition behavior: partitioned nodes should pause local agent execution or continue independently?

---

## Additional Enhancements

### Persistent Storage (SQLite)

**Motivation**: Episodic ledger, audit log, and panic log are all in-memory and lost on restart.

**Implementation**:

1. Add `rusqlite` dependency.
2. Create `src/storage.rs`:
   ```rust
   pub struct PersistentStore {
       conn: rusqlite::Connection,
   }

   impl PersistentStore {
       pub fn new(path: &str) -> Result<Self, rusqlite::Error>;
       pub fn append_episodic(&self, entry: &EpisodicEntry) -> rusqlite::Result<()>;
       pub fn query_episodic(&self, pid: Option<&str>, limit: usize) -> Vec<EpisodicEntry>;
       pub fn append_audit(&self, entry: &AuditEntry) -> rusqlite::Result<()>;
       pub fn append_panic(&self, entry: &PanicLog) -> rusqlite::Result<()>;
   }
   ```

3. Wire into `agOSKernel` as an optional store — writes go to both memory (fast path) and SQLite (durability).
4. Add startup flag: `--db-path /var/lib/agos/kernel.db`.

**Files**: `agos-kernel/src/storage.rs`, `agos-kernel/Cargo.toml`.

---

### Rate Limiting and QoS

**Motivation**: Prevent a single agent from flooding the kernel with requests.

**Implementation**:

1. Add a `RateLimiter` struct using `Governor` crate:
   ```rust
   use governor::{RateLimiter as GovLimiter, Quota, clock::DefaultClock};
   use std::num::NonZeroU32;

   pub struct AgentRateLimiter {
       per_process: HashMap<String, GovLimiter<DefaultClock>>,
       default_quota: Quota,
   }
   ```

2. Check in each REST handler before delegating to the kernel:
   ```rust
   if let Err(_) = rate_limiter.check(pid) {
       return Json(json!({"error": "rate limit exceeded"}));
   }
   ```

3. Default: 100 requests/min per process, configurable via `manifest.json`:
   ```json
   "limits": { "max_tokens": 5000, "max_subagents": 3, "rate_limit": 100 }
   ```

**Files**: New file in `agos-kernel/`, or integrate into `scheduler.rs`.

---

### Token Accounting with Real Models

**Motivation**: The current UMI stub drivers use synthetic token counts (`prompt.len() * token_cost`). Real model APIs (OpenAI, Anthropic, Ollama) return actual token usage.

**Implementation**: In each real driver's `reason()`:
```rust
let usage = resp.get("usage").ok_or("no usage")?;
let prompt_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0);
let completion_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);
let total = prompt_tokens + completion_tokens;
d.total_tokens.fetch_add(total, Ordering::Relaxed);
```

**Files**: `agos-kernel/src/umi.rs` — individual driver `reason()` implementations.

---

### OpenTelemetry Instrumentation

**Motivation**: Trace requests across the kernel for debugging and performance analysis.

**Implementation**:

1. Add `opentelemetry`, `opentelemetry-otlp` dependencies.
2. Create `src/telemetry.rs`:
   ```rust
   use opentelemetry::*;

   pub fn init_tracer() -> Result<(), opentelemetry::errors::OpenTelemetryError> {
       global::set_text_map_propagator(opentelemetry_otlp::Propagator::new());
       let tracer = opentelemetry_otlp::new_pipeline()
           .install_simple()?;
       // Use with tracing: tracing_opentelemetry::layer()
       Ok(())
   }
   ```

3. Add `#[instrument]` to each `api.rs` handler.
4. Export to Jaeger or Grafana Tempo.

**Files**: `agos-kernel/src/telemetry.rs`, `agos-kernel/Cargo.toml`, `agos-kernel/src/main.rs`.

---

### Kubernetes Operator

**Motivation**: Deploy agOS as a cloud-native service on Kubernetes.

**Implementation**:

1. `Dockerfile`:
   ```dockerfile
   FROM rust:1.75 AS build
   WORKDIR /app
   COPY . .
   RUN cargo build --release

   FROM debian:bookworm-slim
   COPY --from=build /app/target/release/agos-kernel /usr/local/bin/
   COPY --from=build /app/aapps /aapps
   EXPOSE 7400
   CMD ["agos-kernel"]
   ```

2. `deploy/k8s/`:
   - `deployment.yaml` — kernel pod, liveness/readiness probes on `/health`.
   - `service.yaml` — ClusterIP for internal, NodePort/LoadBalancer for external.
   - `configmap.yaml` — `RUST_LOG=info`, `--db-path` flag.
   - `hpa.yaml` — Horizontal Pod Autoscaler based on memory/cpu.

3. (Optional) `deploy/k8s/operator/` — a Rust or Go operator that manages `.aapp` deployments as custom resources.

**Files**: New `deploy/` directory.

---

### Pluggable Scheduler Policies

**Motivation**: Different workloads need different scheduling — batch processing needs FIFO, interactive agents need priority preemption, research needs fair sharing.

**Implementation**:

1. Define a trait:
   ```rust
   #[async_trait]
   pub trait SchedulePolicy: Send + Sync {
       async fn select_next(&self, processes: &[CognitiveProcessContext]) -> Option<usize>;
       async fn allocate_tokens(&self, ctx: &CognitiveProcessContext, complexity: f64) -> Result<u64, String>;
   }
   ```

2. Implement policies:
   - `PriorityPreemption` — current behavior (System 1/2).
   - `RoundRobin` — fair share across all processes.
   - `FIFO` — queue order.
   - `TokenAware` — processes with more remaining quota get priority.

3. `TokenScheduler` holds `Box<dyn SchedulePolicy>`, swappable at runtime.
4. Expose via `POST /scheduler/policy { "policy": "round_robin" }`.

**Files**: `agos-kernel/src/scheduler.rs` (add trait + implementations), `agos-kernel/src/api.rs` (new route).

---

### Sub-Agent Orchestration DSL

**Motivation**: The current `logic_graphs.json` state machine is limited to flat transitions. A DSL would allow hierarchical sub-agent orchestration with parallel execution, joins, and timeouts.

**Design** (YAML):

```yaml
workflow: financial_audit
steps:
  - id: validate_balance
    agent: BalanceChecker
    timeout: 30s
    on_success: verify_transactions
    on_failure: flag_anomaly

  - id: verify_transactions
    agent: TransactionVerifier
    parallel:
      - agent: FraudDetector
      - agent: ComplianceChecker
    join: all
    on_success: generate_report

  - id: generate_report
    agent: ReportGenerator
    on_success: __terminal__
```

**Implementation**:

1. Parse DSL in `AAppLoader` alongside `logic_graphs.json`.
2. `SubAgentOrchestrator` struct in `kernel.rs` manages the DAG execution.
3. Each step spawns a sub-agent process (child PID).
4. Parallel steps spawn multiple children and wait for all to complete (join) or any to complete (race).

**Files**: `agos-kernel/src/orchestrator.rs`, DSL parsing in `sandbox.rs`.

---

### Structured Audit Export

**Motivation**: The audit log is in-memory JSON. Compliance use cases require export to S3, Splunk, or SIEM systems.

**Implementation**:

1. Add `POST /security/audit/export` with parameters:
   ```json
   { "format": "jsonl" | "csv", "since": "2026-01-01T00:00:00Z", "sink": "stdout" | "s3" }
   ```

2. Implement `AuditExporter` trait:
   ```rust
   #[async_trait]
   pub trait AuditExporter {
       async fn export(&self, entries: &[AuditEntry]) -> Result<(), String>;
   }
   ```

3. Implementations: `StdoutExporter`, `S3Exporter` (using `aws-sdk-s3`), `SplunkExporter` (HTTP Event Collector).

**Files**: `agos-kernel/src/audit_export.rs`, `agos-kernel/src/api.rs`.

---

### Secure .aapp Signing

**Motivation**: Ensure `.aapp` packages haven't been tampered with since authoring.

**Design**:

```
.aapp directory:
├── manifest.json
├── logic_graphs.json
├── tools/
├── validation_tests/
└── signature.sig      ← ED25519 signature of all other files
```

**Implementation**:

1. On install, the kernel checks `signature.sig` against a trusted public key (`--trusted-key` flag).
2. Signing tool: `agos-sign` CLI that walks a `.aapp` directory, hashes all files, signs with a private key.
3. Rejection: if signature is missing or invalid, the install fails with `"signature verification failed"`.

**Files**: New CLI tool `agos-sign/`, verification logic in `sandbox.rs`.

---

## Feature Prioritization

| Feature | Effort | Impact | Dependencies | Recommended Order |
|---------|--------|--------|-------------|-------------------|
| Persistent Storage (SQLite) | 2 days | High — data survives restart | None | 1 |
| Token Accounting with Real Models | 1 day | High — accurate budgeting | UMI drivers (existing) | 2 |
| Rate Limiting and QoS | 1 day | Medium — production hardening | None | 3 |
| Pluggable Scheduler Policies | 3 days | Medium — scheduling flexibility | None | 4 |
| Secure .aapp Signing | 2 days | Medium — supply chain security | None | 5 |
| Structured Audit Export | 2 days | Medium — compliance use cases | Persistent Storage | 6 |
| **Phase 6**: Episodic Compression Daemon | 3 days | Medium — memory management | None | 7 |
| Sub-Agent Orchestration DSL | 5 days | High — agent composability | None | 8 |
| **Phase 7**: Z3 SMT Solver | 5 days | Medium — formal verification | None | 9 |
| OpenTelemetry Instrumentation | 2 days | Low — observability | None | 10 |
| **Phase 8**: WebAssembly Sandbox | 2 weeks | High — secure tool execution | wasmtime | 11 |
| Kubernetes Operator | 3 days | Medium — cloud deployment | Dockerfile | 12 |
| **Phase 9**: Embodiment Layer | 3 weeks | High — robotics use cases | UMI, WASM sandbox | 13 |
| **Phase 10**: Decentralized Grids | 4 weeks | High — scale-out architecture | Persistent Storage | 14 |

---

## Design Principles for New Features

1. **Backward compatibility** — existing `.aapp` packages and REST clients must continue to work without modification.
2. **Opt-in complexity** — default configuration should be the simplest possible (in-memory, single-node). Advanced features (persistence, clustering) are opt-in.
3. **No panics in library code** — all error paths return `Result<T, String>`. The only `unwrap()` is in `main.rs` during bootstrap.
4. **Lock discipline** — never hold `std::sync::RwLock` across `.await`. Use `tokio::sync::RwLock` when async I/O is inside the critical section.
5. **Test first** — every new feature must include unit tests for the core logic and an integration step in `DemoOrchestrator`.
6. **Metric every operation** — every new subsystem must expose counters (calls, latency, errors) via a `get_metrics()` method.
7. **Configuration over code** — knobs that may need tuning (intervals, limits, timeouts) should be runtime configurable, not hardcoded.
8. **One file per concern** — a new subsystem gets its own file (or files). No 2000+ line files.
