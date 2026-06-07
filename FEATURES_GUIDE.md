# Features Guide

This guide documents all 11 phases of the agOS development roadmap — **Phases 1–5 (completed)** and **Phases 6–11 (completed)** — plus additional enhancements beyond the core roadmap. Each entry includes the motivation, design overview, implementation steps, files to modify, testing strategy, and open questions. Use this as a blueprint when picking up the next phase of work.

---

## Table of Contents

- [Phase 1 — Deterministic Firmware Scaffolding](#phase-1--deterministic-firmware-scaffolding)
- [Phase 2 — Unified Model Interface](#phase-2--unified-model-interface)
- [Phase 3 — Hierarchical Cognitive Memory + cIPC Bus](#phase-3--hierarchical-cognitive-memory--cipc-bus)
- [Phase 4 — Ring 0 Neuro-Symbolic Guard Layer](#phase-4--ring-0-neuro-symbolic-guard-layer)
- [Phase 5 — Sandbox Installer + Integration Smoke Test](#phase-5--sandbox-installer--integration-smoke-test)
- [Phase 6 — Episodic Compression Daemon (Completed)](#phase-6--episodic-compression-daemon)
- [Phase 7 — Native Z3 SMT Solver (Completed)](#phase-7--native-z3-smt-solver)
- [Phase 8 — WebAssembly Sandbox (Completed)](#phase-8--webassembly-sandbox)
- [Phase 9 — Embodiment Layer (VLA Driver Pipeline) (Completed)](#phase-9--embodiment-layer-vla-driver-pipeline)
- [Phase 10 — Decentralized Cognitive Grids (Completed)](#phase-10--decentralized-cognitive-grids)
- [Phase 11 — Unified Robotics Interface + Hardware Control (Completed)](#phase-11--unified-robotics-interface--hardware-control)
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

## Phase 6 — Episodic Compression Daemon (Completed)

### Motivation

The episodic ledger is an in-memory `Vec<EpisodicEntry>` that grows unboundedly with every system event (spawn, kill, cIPC, panic, etc.). Without a background eviction and compression strategy, the kernel will exhaust memory under sustained load.

### What Was Built

A background tokio task runs on a configurable interval and performs age-based compression:

- **`EpisodicConfig`** (`src/memory.rs`) — config struct with `compression_interval_secs` (default: 60), `max_entries_before_compress` (default: 100), and `entry_ttl_secs` (default: 300).
- **`compress_episodic()`** on `HierarchicalMemoryManager` — partitions the ledger into fresh vs stale entries by timestamp, prunes stale entries, writes summaries to L2.
- **Background daemon** (`src/main.rs`) — `tokio::spawn` loop that calls `compress_episodic()` on the configured interval with `MissedTickBehavior::Skip`.
- **Manual trigger** via `POST /memory/episodic/compress` REST endpoint.
- **`compressed_entries`** counter added to `MemoryStats`.

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

### Key Files

| File | Change |
|------|--------|
| `agos-kernel/src/memory.rs` | `EpisodicConfig`, `compress_episodic()`, `compressed_entries` stats field |
| `agos-kernel/src/main.rs` | Background `tokio::spawn` daemon |
| `agos-kernel/src/api.rs` | `POST /memory/episodic/compress` route |

### Testing

```bash
cargo test -- memory
# 9 tests including:
# - test_compress_episodic_no_stale — TTL=0 prunes everything
# - test_compress_episodic_with_stale — TTL=3600 keeps fresh entries
# - test_compress_episodic_partial — verifies progressive compression
```

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

## Phase 9 — Embodiment Layer (VLA Driver Pipeline) (Completed)

### Motivation

Agents must bridge the gap between cognitive reasoning and real-world interaction. The asks.txt vision describes an agOS that can accept natural language, video, text, and sensory inputs (all 5 human senses), then build and orchestrate agents to achieve user goals. The Embodiment Layer implements this as a **VLA (Vision-Language-Action) driver pipeline** — a multimodal goal-processing workflow with perception, understanding, planning, execution, and feedback stages.

### What Was Built

- **`EmbodimentLayer`** (`src/embodiment.rs`) — orchestrates the full VLA pipeline:
  - **`submit_goal()`** — accepts a natural language statement + optional sensory inputs (image, audio, video, haptic, olfactory, gustatory, text)
  - **`run_pipeline()`** — async pipeline: Perception → Understanding (via UMI) → Planning → Execution
  - **`provide_clarification()`** — if the plan needs disambiguation, accepts follow-up text and re-plans (up to 3 rounds). State validation guards reject calls on non-`NeedsClarification` goals. Round counter increments atomically. Status transitions correctly from `Understanding` → `Executing` or back to `NeedsClarification`. Each clarification event is logged to episodic memory.
  - **`execute_plan()`** — spawns sub-agents for each task via `kernel.spawn_process()`, logs to episodic memory
  - **`process_sensory_input()`** — wraps raw sensor data into typed `SensoryInput` structs
  - **`available_sensors()`** — reports all 7 supported modalities
  - **`list_agent_apps()`** — searches available `.aapp` packages for task composition
  - **`detect_figures_of_speech()`** — heuristic scanner that flags 20+ English idioms/metaphors (`"pull the trigger"`, `"drill down"`, `"low-hanging fruit"`, etc.) plus modality-aware casual phrasing detection for voice/text inputs
  - **`resolve_with_umi()`** — consults UMI *before* asking the user; parses `RESOLVED: "<phrase>" → <meaning>` from the model and marks ambiguities as `resolved_by: "umi"` vs `""`
  - **`format_ambiguity_context()`** — produces `[Resolved]` / `[Unresolved]` prefixed strings so `create_plan()` can pass resolved items silently and only flag unresolved ones for clarification

- **New types**:
  - `SemanticAmbiguity { phrase, meaning, context, resolved_by }`
  - `FigureOfSpeechKind { pattern: str, interpretation: str, confidence: f64 }`
  - `EmbodimentGoal.detected_ambiguities: Vec<SemanticAmbiguity>`

- **Pipeline stages**:
  ```
  Goal text + sensory inputs
          │
          ▼
  ┌────────────────────┐
  │  1. Perception     │  ← Parse modalities, extract metadata
  ├────────────────────┤
  │  2. Understanding  │  ← detect_figures_of_speech → resolve_with_umi
  ├────────────────────┤
  │  3. Planning       │  ← create_plan (only flags [Unresolved] ambiguities)
  ├────────────────────┤
  │  4. Clarification  │  ← provide_clarification (state-guarded, round-capped)
  ├────────────────────┤
  │  5. Execution      │  ← Spawn sub-agents, monitor progress
  ├────────────────────┤
  │  6. Feedback       │  ← Log to episodic memory, return results
  └────────────────────┘
  ```

- **REST endpoints** (behind `#[cfg(feature = "embodiment")]`):
  | Endpoint | Method | Description |
  |----------|--------|-------------|
  | `/embodiment/goal` | POST | Submit a goal with optional sensory inputs |
  | `/embodiment/goals` | GET | List all goals |
  | `/embodiment/goal/{id}` | GET | Get goal status |
  | `/embodiment/goal/{id}/plan` | GET | Get action plan |
  | `/embodiment/goal/{id}/clarify` | POST | Provide clarification |
  | `/embodiment/goal/{id}/execute` | POST | Execute plan |
  | `/embodiment/sensors` | GET | List available sensors |
  | `/embodiment/apps` | GET | List available agent apps |

### Key Files

| File | Purpose |
|------|---------|
| `agos-kernel/src/embodiment.rs` | `EmbodimentLayer`, goal/pipeline/plan types, figure-of-speech detection, UMI resolution |
| `agos-kernel/src/lib.rs` | `pub mod embodiment` behind `embodiment` feature |
| `agos-kernel/src/kernel.rs` | `pub embodiment: EmbodimentLayer` field |
| `agos-kernel/src/api.rs` | 8 `/embodiment/*` routes behind cfg gate |
| `agos-kernel/Cargo.toml` | `embodiment = []` feature flag |

### Testing

```bash
# With feature flag:
cargo test --features embodiment
# 22 embodiment tests + 9 memory tests = 31 total

# All features:
cargo test --features "z3-solver,wasm-sandbox,embodiment"
# 46 tests total (22 embodiment + 9 memory + 2 sandbox + 4 Z3 + 9 grid)
```

### Key Design Decisions

- Feature-gated (`embodiment = []`) — zero overhead when disabled.
- Sync `EmbodimentLayer` with `std::sync::Mutex` for internal state; pipeline methods are `async` because they call `UMI::reason()`.
- Sensory inputs are abstract `String` payloads — actual encoding (base64 images, WAV audio) handled by the caller.
- Agent app search returns known `.aapp` names — can be extended with real filesystem scanning.
- Clarification is handled via a round counter (max 3 rounds) with **state validation** — only `NeedsClarification`-status goals accept clarification; status transitions are correctly `Understanding → Executing/NeedsClarification`.
- Figure-of-speech detection uses a **heuristic scanner** (string patterns with `contains()`), not a model — fast, deterministic, suitable for a first pass.
- UMI resolution prompt instructs the model to "minimise user clarification rounds while avoiding incorrect actions" — encourages decisive resolution when confident, explicit ambiguity flagging when not.
- `format_ambiguity_context()` produces `[Resolved]` / `[Unresolved]` prefixed strings so downstream code can trivially filter by prefix.

---

## Phase 10 — Decentralized Cognitive Grids (Completed)

### Motivation

Single-node agOS is limited by available compute. Decentralized Cognitive Grids enable **multi-node agent orchestration** — agents on one node can cIPC to agents on another node, migrate between nodes, and coordinate globally. No external dependencies (etcd/Consul) — nodes discover each other manually via REST and maintain heartbeats for liveness.

### What Was Built

- **`CognitiveGrid`** (`src/grid.rs`) — core grid state and operations:
  - **Peer discovery** — `add_peer(node_id, address)` registers a peer; `list_peers()` returns all; `remove_peer()` drops a peer.
  - **Heartbeat tracking** — `record_heartbeat(node_id)` timestamps receipt; `stale_peers(threshold_secs)` returns peers with no heartbeat within the window.
  - **Agent migration** — `freeze_process(pid, context, agent_app)` serialises process state into a `MigratingProcess` payload; `receive_process(incoming)` deserialises and returns `CognitiveProcessContext` for the kernel to register.
  - **PID resolution** — `resolve_pid(pid)` returns `ResolveResult::Local` or `ResolveResult::Remote(node_id)` based on peer-stored PID ownership.
  - **Metrics** — `stats()` returns `GridStats { peer_count, message_count }`.

- **Key types**:
  ```rust
  pub struct PeerInfo { pub node_id: String, pub address: String, pub status: PeerStatus, pub last_seen: String }
  pub enum PeerStatus { Online, Stale, Offline }
  pub struct MigratingProcess { pub pid, pub agent_app, pub state }
  pub enum GridMessage { Join { node_id, address }, Heartbeat { node_id }, Migrate { pid, target_node }, ... }
  pub struct GridStats { pub peer_count: usize, pub message_count: u64 }
  ```

- **Kernel integration** (`kernel.rs`):
  - `pub grid: CognitiveGrid` field (NOT feature-gated — always compiled)
  - `agOSKernel::new(umi, listen_addr)` — constructor takes a listen address for the node ID
  - `migrate_process(pid, target_node)` — freezes the process via `grid.freeze_process`, logs a `PROCESS_MIGRATED` episodic event, kills the local process
  - `receive_process(incoming)` — calls `grid.receive_process`, registers the new PID on the scheduler, logs a `PROCESS_RECEIVED` episodic event

- **REST endpoints**:
  | Endpoint | Method | Description |
  |----------|--------|-------------|
  | `/grid/join` | POST | Register peer node on the grid |
  | `/grid/peers` | GET | List known peers |
  | `/grid/stats` | GET | Grid statistics |
  | `/grid/heartbeat` | POST | Record heartbeat from peer |
  | `/grid/migrate` | POST | Migrate agent to another node |
  | `/grid/receive` | POST | Accept migrated agent |
  | `/grid/cipc/forward` | POST | Forward cIPC message from another node |

### Key Files

| File | Change |
|------|--------|
| `agos-kernel/src/grid.rs` | New file — `CognitiveGrid`, `PeerInfo`, `PeerStatus`, `MigratingProcess`, `GridMessage`, `GridStats` |
| `agos-kernel/src/lib.rs` | Added `pub mod grid` (no feature gate) |
| `agos-kernel/src/kernel.rs` | Added `pub grid: CognitiveGrid`; constructor takes `listen_addr`; `migrate_process()`, `receive_process()` |
| `agos-kernel/src/api.rs` | 7 `/grid/*` routes |
| `agos-kernel/src/main.rs` | `agOSKernel::new(umi, "127.0.0.1:7400")` |

### Testing

```bash
# Grid unit tests (always run, no feature gate)
cargo test -- grid
# 9 tests: add/list/remove peers, heartbeat, stale detection, freeze/receive, PID resolution, stats

# Full suite:
cargo test --features "z3-solver,wasm-sandbox,embodiment"
# 46 tests total (22 embodiment + 9 memory + 2 sandbox + 4 Z3 + 9 grid)
```

### Key Design Decisions

- **No feature gate** — `CognitiveGrid` is always compiled (core subsystem), unlike Z3/WASM/Embodiment. This means grid endpoints are always available.
- **P2P discovery** — no etcd/Consul dependency. Nodes add each other manually via `/grid/join`. Heartbeats are passive (recorded on receipt). A background daemon for liveness checks can be added later.
- **`receive_process()` returns `CognitiveProcessContext`** without registering — the kernel wrapper `agOSKernel::receive_process()` handles registration and episodic logging, keeping the grid module agnostic to kernel internals.
- **`resolve_pid()`** returns an enum (`Local` vs `Remote(node_id)`), allowing future cIPC route extension to forward messages to remote nodes via `/grid/cipc/forward`.

### Open Questions

- Conflict resolution: two nodes claim the same PID. Use `NodeId + PID` as the global identifier?
- Consistency model: eventual consistency for the episodic ledger? Strong consistency for process state?
- Network partition behavior: partitioned nodes should pause local agent execution or continue independently?

---

## Phase 11 — Unified Robotics Interface + Hardware Control (Completed)

### Motivation

agOS must bridge the gap between cognitive reasoning and physical hardware. Just as UMI abstracts AI models through four syscalls, the **Unified Robotics Interface (URI)** abstracts hardware sensors and actuators through a uniform interface. The **Controller** module provides real-time control primitives — PID loops, trajectory planning, safety monitors, and coordinate transforms — needed for robotics, automation, and IoT use cases.

### What Was Built

**`uri.rs` — UnifiedRoboticsInterface** (`src/uri.rs`):
- **Hardware driver registry** — builtin (compile-time) and runtime (dynamic) drivers, each declaring capabilities (e.g., `sensor:temperature`, `actuator:servo`).
- **Device state tracking** — per-device last value, timestamp, unit, and active driver mapping.
- **Data transforms** — composable pipelines of `Normalize`, `Scale`, `Threshold`, `MovingAverage`, and `Custom` steps.
- **4 built-in drivers**: `SimulatedTemperature`, `SimulatedServo`, `SimulatedDistance`, `SimulatedIMU`.
- **10 REST endpoints** under `/uri/*`.

**`controller.rs` — ControllerManager** (`src/controller.rs`):
- **PID controllers** — configurable gains (Kp, Ki, Kd), setpoint, integral windup, output clamping, per-step computation.
- **Trajectory planning** — waypoint-based plans with linear interpolation, current-index tracking, start/step execution model.
- **Safety monitors** — four condition types (`ThresholdAbove`, `ThresholdBelow`, `RateOfChange`, `Window`) with threshold checks, triggered state tracking.
- **Emergency stop** — device-scoped estop with log and clear operations.
- **Coordinate transforms** — named 4×4 matrix transforms for robot kinematics.
- **Safety event log** — append-only event store with clearable ESTOP events.
- **17 REST endpoints** under `/controller/*`.

### Design

```
URI (Unified Robotics Interface):
  ┌─────────────────────┐
  │  Hardware Drivers    │  ← builtin (static) + runtime (dynamic)
  │  s                   │
  │  s                   │
  │     SimTemp ──► sensor:temperature
  │     SimServo ──► actuator:servo
  │     SimDist  ──► sensor:distance
  │     SimIMU   ──► sensor:imu
  └─────────┬───────────┘
            │ sense() / actuate()
            ▼
  ┌─────────────────────┐
  │   Device Registry   │  ← device_id → (driver, last_value, unit)
  └─────────┬───────────┘
            │
  ┌─────────▼───────────┐
  │  Data Transforms    │  ← Normalize → Scale → Threshold → MA
  └─────────────────────┘

Controller (Hardware Control Loops):
  ┌─────────────────────┐
  │   PID Controllers   │  ← Kp, Ki, Kd, setpoint, step()
  ├─────────────────────┤
  │  Trajectory Plans   │  ← waypoints, interpolation, start/step
  ├─────────────────────┤
  │  Safety Monitors    │  ← ThresholdAbove/Below, RateOfChange, Window
  ├─────────────────────┤
  │  Emergency Stop     │  ← per-device estop + clear
  ├─────────────────────┤
  │  Coord Transforms   │  ← 4×4 matrices, named registry
  ├─────────────────────┤
  │  Safety Event Log   │  ← append-only, clearable
  └─────────────────────┘
```

### REST Endpoints

**URI** (10 endpoints):

| Method | Path | Description |
|--------|------|-------------|
| POST | `/uri/sense` | Read sensor value via driver |
| POST | `/uri/actuate` | Write actuator value via driver |
| POST | `/uri/transform` | Apply a data transform pipeline |
| POST | `/uri/status` | Query driver/device status |
| POST | `/uri/switch` | Switch active driver for a device |
| GET | `/uri/metrics` | Per-driver call/error metrics |
| GET | `/uri/drivers` | List registered drivers |
| GET | `/uri/drivers/{name}` | Get driver metadata |
| POST | `/uri/drivers/register` | Register runtime hardware driver |
| POST | `/uri/drivers/unregister` | Unregister runtime hardware driver |

**Controller** (17 endpoints):

| Method | Path | Description |
|--------|------|-------------|
| POST | `/controller/pid` | Create PID controller |
| GET | `/controller/pid` | List PID controllers |
| POST | `/controller/pid/setpoint` | Update PID setpoint |
| POST | `/controller/pid/step` | Execute one PID control step |
| POST | `/controller/trajectory` | Create trajectory plan |
| GET | `/controller/trajectory` | List trajectory plans |
| POST | `/controller/trajectory/start` | Start trajectory execution |
| POST | `/controller/trajectory/step` | Advance trajectory one waypoint |
| POST | `/controller/safety` | Create safety monitor |
| POST | `/controller/safety/check` | Run safety check on a device |
| POST | `/controller/safety/estop` | Trigger emergency stop |
| POST | `/controller/safety/estop/clear` | Clear emergency stop |
| GET | `/controller/safety/monitors` | List safety monitors |
| GET | `/controller/safety/events/{device_id}` | Get safety events for a device |
| POST | `/controller/transform` | Add coordinate transform |
| GET | `/controller/transform` | List coordinate transforms |
| GET | `/controller/stats` | Controller statistics |

### Key Files

| File | Purpose |
|------|---------|
| `agos-kernel/src/uri.rs` | `UnifiedRoboticsInterface`, `HardwareDriver`, `DeviceState`, `DataTransform` |
| `agos-kernel/src/controller.rs` | `ControllerManager`, `PIDController`, `TrajectoryPlan`, `SafetyMonitor`, `SafetyEvent` |
| `agos-kernel/src/lib.rs` | `pub mod uri`, `pub mod controller` behind `uri` feature |
| `agos-kernel/src/kernel.rs` | `pub uri`, `pub controller` fields behind `#[cfg(feature = "uri")]` |
| `agos-kernel/src/api.rs` | 10 `/uri/*` + 17 `/controller/*` routes behind cfg gate |
| `agos-kernel/Cargo.toml` | `uri = []` feature flag |

### Testing

```bash
# With feature flag:
cargo test --features uri

# All features:
cargo test --features "z3-solver,wasm-sandbox,embodiment,uri"
# 56 tests total
```

### Key Design Decisions

- **Single feature flag (`uri`)** — both URI and Controller share the same feature, since controller depends on URI concepts (device IDs, drivers).
- **Sync methods** — both URI and Controller methods are synchronous (no async). Hardware operations are expected to be fast (register reads, actuator writes). Async can be added per-driver if needed.
- **`std::sync::RwLock` for URI** — concurrent reads for sense/status/metrics are common; writes (register/unregister/switch) are rare.
- **`std::sync::Mutex` for Controller** — reads and writes are both infrequent and short; a single Mutex avoids the complexity of RwLock while being sufficient for demo-scale loads.
- **Built-in simulated drivers** — 4 built-in drivers provide realistic test data (temperature, servo angle, distance, IMU orientation) without external hardware.
- **Runtime driver registration** — third-party hardware drivers can be registered dynamically (e.g., GPIO via sysfs, I2C, SPI) without recompiling the kernel.
- **Safety loop is app-driven** — the kernel does not run a background safety monitor. Apps call `/controller/safety/check` before critical operations. A background daemon can be added later.
- **Event log is append-only** — ESTOP events are never deleted, only marked `cleared: true`. This provides an audit trail for safety incidents.

### Open Questions

- Should there be a background safety daemon that polls monitored devices on a timer?
- Should the kernel support hot-plug detection for hardware devices?
- How to handle driver-to-device mapping when multiple drivers support the same capability?
- Should coordinate transforms support composition (multiply two transforms, invert)?

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

### OpenTelemetry Instrumentation (Completed)

**Motivation**: Trace requests across the kernel for debugging and performance analysis.

**What Was Built**:

- **`telemetry.rs`** (`src/telemetry.rs`) — initialises an OTLP tracer via `opentelemetry_otlp`, sets the global text map propagator, and returns a `tracing_opentelemetry` layer that layers on top of the `tracing-subscriber` registry.
- **`init_telemetry()`** — called from `main.rs` right after `tracing-subscriber` init; registers the OTLP exporter and an optional `ShutdownGuard` that flushes spans on drop.
- **`#[instrument]`** annotations added to all `api.rs` handler functions, UMI syscall wrappers, and kernel orchestration methods (`spawn_process`, `kill_process`, `kernel_panic`, `sandbox_install`, `sandbox_launch`, `migrate_process`, `receive_process`).
- **Graceful shutdown** — the `shutdown_guard` flushes pending spans when the kernel receives SIGTERM or SIGINT.
- Configuration via environment variables: `OTLP_ENDPOINT` (defaults to `http://localhost:4317`), `AGOS_SERVICE_NAME` (defaults to `agos-kernel`).

**Key Files**:

| File | Purpose |
|------|---------|
| `agos-kernel/src/telemetry.rs` | Tracer init, `#[instrument]` macro usage, shutdown guard |
| `agos-kernel/Cargo.toml` | Added `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` |
| `agos-kernel/src/main.rs` | `init_telemetry()` call in boot sequence |

**Configuration**:

```bash
AGOS_SERVICE_NAME=agos-kernel OTLP_ENDPOINT=http://jaeger:4317 cargo run
```

---

### Kubernetes Operator (Completed)

**Motivation**: Deploy agOS as a cloud-native service on Kubernetes.

**What Was Built**:

- **`deploy/k8s/deployment.yaml`** — Kubernetes Deployment with 1 replica (configurable), liveness/readiness probes on `/health`, resource requests/limits, environment variables from ConfigMap.
- **`deploy/k8s/service.yaml`** — ClusterIP service on port 7400, with annotations for Prometheus scraping.
- **`deploy/k8s/configmap.yaml`** — `RUST_LOG=info` and `AGOS_SERVICE_NAME=agos-kernel`.
- **`deploy/k8s/hpa.yaml`** — Horizontal Pod Autoscaler targeting 50% CPU / 70% memory utilization, min 1 / max 10 replicas.
- All manifests include `apiVersion`, `kind`, `metadata.labels`, and `spec.template.metadata.labels` consistent with Kubernetes best practices.

**Key Files**:

| File | Purpose |
|------|---------|
| `deploy/k8s/deployment.yaml` | Kernel pod spec with probes and resource limits |
| `deploy/k8s/service.yaml` | ClusterIP service definition |
| `deploy/k8s/configmap.yaml` | Environment configuration |
| `deploy/k8s/hpa.yaml` | Autoscaling policy |

**Deploy**:

```bash
kubectl apply -f deploy/k8s/
```

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

### Sub-Agent Orchestration DSL (Completed)

**Motivation**: The current `logic_graphs.json` state machine is limited to flat transitions. A DSL would allow hierarchical sub-agent orchestration with parallel execution, joins, and timeouts.

**What Was Built**:

- **`orchestrator.rs`** (`src/orchestrator.rs`) — `SubAgentOrchestrator` struct that manages a map of workflow instances (`HashMap<String, WorkflowInstance>`). Each instance tracks the `WorkflowDefinition` (parsed DSL), its current step, per-step status (`Pending`, `Running`, `Succeeded`, `Failed`, `TimedOut`), child PIDs, and timestamps.
- **DSL parsing** — the `WorkflowDefinition` YAML/JSON format is parsed in `sandbox.rs` as part of `AAppLoader`. A new optional field `workflow` is extracted from the `.aapp` directory alongside `logic_graphs.json`. The DSL supports:
  - Sequential steps with `on_success` / `on_failure` transitions
  - Parallel branches (`parallel: [{agent: X}, {agent: Y}]`) with `join: all` or `join: any`
  - Timeouts per step (`timeout: 30s`)
  - `__terminal__` sentinel for workflow completion
- **Sandbox integration** — `InstallResult` now includes a `has_workflow: bool` field; the sandbox loader checks for a `workflow` key in `manifest.json` and stores it during install.
- **REST endpoints**:
  - `POST /workflow/run {name, aapps_path}` — loads the `.aapp`, parses its workflow DSL, creates a `WorkflowInstance`, spawns the first step as a sub-agent, and returns the workflow ID.
  - `GET /workflow/status/:id` — returns the current step, overall status, child PIDs, and step-level details.
- **Kernel wiring** — `agOSKernel` holds a `pub orchestrator: SubAgentOrchestrator` field; the workflow endpoints delegate to orchestrator methods which call `spawn_process` for each step.
- A workflow step's agent is expected to be another `.aapp` package name; the orchestrator calls `sandbox_install` + `sandbox_launch` under the hood.

**Key Files**:

| File | Purpose |
|------|---------|
| `agos-kernel/src/orchestrator.rs` | `SubAgentOrchestrator`, `WorkflowDefinition`, `WorkflowInstance`, DSL types |
| `agos-kernel/src/sandbox.rs` | Workflow DSL extraction in `AAppLoader`, `has_workflow` field |
| `agos-kernel/src/kernel.rs` | `pub orchestrator` field |
| `agos-kernel/src/api.rs` | `POST /workflow/run`, `GET /workflow/status/:id` routes |
| `agos-kernel/src/lib.rs` | `pub mod orchestrator` |

**Example DSL** (in `manifest.json`):

```json
{
  "name": "Financial Audit",
  "workflow": {
    "steps": [
      {"id": "validate_balance", "agent": "BalanceChecker", "timeout": 30, "on_success": "verify_transactions", "on_failure": "flag_anomaly"},
      {"id": "verify_transactions", "agent": "TransactionVerifier", "on_success": "generate_report"},
      {"id": "generate_report", "agent": "ReportGenerator", "on_success": "__terminal__"}
    ]
  }
}
```

---

### Structured Audit Export (Completed)

**Motivation**: The audit log is in-memory JSON. Compliance use cases require export to S3, Splunk, or SIEM systems.

**What Was Built**:

- **`audit_export.rs`** (`src/audit_export.rs`) — defines the `AuditExporter` async trait and three implementations:
  - `StdoutExporter` — formats entries as JSONL and prints to stdout.
  - `JsonlFileExporter` — writes JSONL to a local file path.
  - `S3Exporter` — formats entries as JSONL and writes to an S3-compatible object store (uses `rusty_s3` for S3 REST API calls).
- **`format_entries(entries, format)`** — supports `"jsonl"` (NDJSON stream of audit objects) and `"csv"` (comma-separated with header row: `timestamp,action,result,reason`).
- **REST endpoint**: `POST /security/audit/export` with request body:
  ```json
  {"format": "jsonl", "since": "2026-06-01T00:00:00Z", "sink": {"type": "s3", "bucket": "agos-audit", "key": "audit/export.jsonl"}}
  ```
  Supported sink types: `"stdout"`, `"jsonl_file"`, `"s3"`.
- **Since filter** — optional ISO 8601 timestamp; only entries after this time are included.
- The endpoint filters in-memory audit entries by `since`, formats them via `format_entries`, then delegates to the selected exporter.

**Key Files**:

| File | Purpose |
|------|---------|
| `agos-kernel/src/audit_export.rs` | `AuditExporter` trait, `StdoutExporter`, `JsonlFileExporter`, `S3Exporter`, `format_entries()` |
| `agos-kernel/src/api.rs` | `POST /security/audit/export` route |
| `agos-kernel/src/lib.rs` | `pub mod audit_export` |

**Testing**:

```bash
curl -X POST http://127.0.0.1:7400/security/audit/export \
  -H 'Content-Type: application/json' \
  -d '{"format": "jsonl", "since": "2026-06-01T00:00:00Z", "sink": {"type": "stdout"}}'
```

---

### Secure .aapp Signing (Completed)

**Motivation**: Ensure `.aapp` packages haven't been tampered with since authoring.

**What Was Built**:

- **`agos-sign/`** — a standalone Rust CLI tool with three subcommands:
  - `gen-key` — generates an ED25519 keypair, writes private key to `private.pem` and public key to `public.pem` (both PEM-encoded).
  - `sign <aapp-dir>` — walks the `.aapp` directory (all files except `signature.sig`), concatenates them deterministically, SHA-256 hashes, signs with ED25519, writes `signature.sig` as raw bytes.
  - `verify <aapp-dir> <public-key-file>` — re-hashes all files, verifies `signature.sig` against the public key.
- **Sandbox verification** — in `sandbox.rs`, during `install`:
  - Checks for `signature.sig` in the `.aapp` directory.
  - If present, hashes the package files and verifies the signature against the public key loaded from the `AGOS_TRUSTED_KEY` environment variable (or skips verification if unset — backward compatible).
  - Rejects install with `"signature verification failed"` if the signature is invalid.
  - Adds `signature_valid: bool` to `InstallResult`.
- **No external dependencies** — uses `ed25519-dalek` (2.x) and `sha2` for the cryptographic operations.

**Key Files**:

| File | Purpose |
|------|---------|
| `agos-sign/src/main.rs` | CLI tool: `gen-key`, `sign`, `verify` subcommands |
| `agos-sign/Cargo.toml` | `ed25519-dalek`, `sha2`, `clap`, `base64` |
| `agos-kernel/src/sandbox.rs` | Signature verification in `AAppLoader`, `signature_valid` field |
| `agos-kernel/Cargo.toml` | `ed25519-dalek`, `sha2` dependencies |

**Usage**:

```bash
# Generate a keypair
cargo run -p agos-sign -- gen-key

# Sign a .aapp package
cargo run -p agos-sign -- sign aapps/financial_auditor

# Run kernel with trusted key
AGOS_TRUSTED_KEY=$(cat public.pem) cargo run -p agos-kernel
```

---

## Feature Prioritization

| Feature | Effort | Impact | Dependencies | Status |
|---------|--------|--------|-------------|--------|
| Persistent Storage (SQLite) | 2 days | High — data survives restart | None | Planned |
| Token Accounting with Real Models | 1 day | High — accurate budgeting | UMI drivers (existing) | Planned |
| Rate Limiting and QoS | 1 day | Medium — production hardening | None | Planned |
| Pluggable Scheduler Policies | 3 days | Medium — scheduling flexibility | None | Planned |
| Secure .aapp Signing | 2 days | Medium — supply chain security | None | ✓ Implemented |
| Structured Audit Export | 2 days | Medium — compliance use cases | None | ✓ Implemented |
| **Phase 6**: Episodic Compression Daemon | ✓ | — | — | ✓ Complete |
| Sub-Agent Orchestration DSL | 5 days | High — agent composability | None | ✓ Implemented |
| **Phase 7**: Z3 SMT Solver | 5 days | Medium — formal verification | None | ✓ Complete |
| OpenTelemetry Instrumentation | 2 days | Medium — observability | None | ✓ Implemented |
| **Phase 8**: WebAssembly Sandbox | 2 weeks | High — secure tool execution | wasmtime | ✓ Complete |
| Kubernetes Operator | 3 days | Medium — cloud deployment | Dockerfile | ✓ Implemented |
| **Phase 9**: Embodiment Layer | 3 weeks | High — robotics use cases | UMI, WASM sandbox | ✓ Complete |
| **Phase 10**: Decentralized Grids | 4 weeks | High — scale-out architecture | Persistent Storage | ✓ Complete |
| **Phase 11**: URI + Hardware Control | 2 weeks | High — hardware abstraction | None | ✓ Complete |

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
