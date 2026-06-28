# ⟨agOS⟩ — Agentic Operating System

**agOS** is a distributed, neuro-symbolic operating system engineered to natively host, schedule, isolate, and orchestrate autonomous, goal-oriented AI agents and cognitive utilities. Traditional OSes (Linux, Windows) are demoted to silicon-management "firmware." agOS sits above them, abstracting **cognitive primitives**—reasoning compute budgets, contextual attention windows, semantic memory arrays, and non-deterministic tool-routing vectors—into a stable, secure, multi-tenant computing platform.

> *"We're not building better model parts. We're building the chassis, transmission, and steering wheel that turns raw AI horsepower into industrial machinery."*

---

## Architecture

```
+-----------------------------------------------------------------------+
|                    HUMAN / INTENT INTERFACE LAYER                     |
+-----------------------------------------------------------------------+
                                    |
                                    v
+-----------------------------------------------------------------------+
|                       agOS SYSTEM KERNEL (Ring 0)                     |
|                                                                       |
|  +--------------------------------+  +-----------------------------+  |
|  |     COGNITIVE SCHEDULER        |  |  HIERARCHICAL MEMORY MGR    |  |
|  | (System 2 / Token Allocation)  |  |  (Context Window Paging)    |  |
|  +--------------------------------+  +-----------------------------+  |
|  +--------------------------------+  +-----------------------------+  |
|  |     MARKETPLACE & GOVERNANCE   |  |   SUB-AGENT ORCHESTRATOR    |  |
|  | (Agent Catalog / Contracts)    |  | (Workflow DSL / Loop Exec)  |  |
|  +--------------------------------+  +-----------------------------+  |
|  +--------------------------------+  +-----------------------------+  |
|  |     TOOL & cIPC ROUTER         |  | NEURO-SYMBOLIC GUARDRAILS   |  |
|  |   (Virtualized Sandboxes)      |  | (Formal SMT Verification)   |  |
|  +--------------------------------+  +-----------------------------+  |
+-----------------------------------------------------------------------+
                                    |
                                    v
+-----------------------------------------------------------------------+
|            UNIFIED MODEL INTERFACE (UMI) DRIVER LAYER                 |
|      [Local Driver]  [Cloud Driver]  [Custom Drivers...]              |
+-----------------------------------------------------------------------+
                                    |
                                    v
+-----------------------------------------------------------------------+
|                 SILICON FIRMWARE / HARDWARE LAYER                     |
|            (Linux Kernel / Hypervisors / NPU / GPU Hardware)           |
+-----------------------------------------------------------------------+
```

### Core Axioms

1. **Abstraction of Cognition** — The fundamental unit of compute is the Inference Cycle, not the CPU instruction cycle.
2. **Deterministic Governance Over Non-Deterministic Execution** — Probabilistic model outputs are constrained by mathematically verifiable state boundaries (Ring 0 Security).
3. **Decoupling of Weights from Logic** — Applications target the standard agOS Cognitive API, independent of specific model architectures or parameter dimensions.

---

## Project Structure

```
agos/
├── agos-kernel/                   # Rust kernel (port 7400)
│   ├── Cargo.toml
│   ├── aapps/                     # .aapp package bundles
│   │   ├── financial_auditor/     # Financial Auditor demo app
│   │   └── accountant/            # Accountant demo app
│   └── src/
│       ├── main.rs                # HTTP server entry point
│       ├── lib.rs                 # Module declarations
│       ├── kernel.rs              # agOSKernel orchestrator
│       ├── scheduler.rs           # TokenScheduler, CognitiveProcessContext
│       ├── memory.rs              # HierarchicalMemoryManager (L1/L2/Vector RAM)
│       ├── cipc.rs                # Cognitive Inter-Process Communication bus
│       ├── security.rs            # Ring 0 SecurityEngine + SMTSolver
│       ├── umi.rs                 # UnifiedModelInterface + driver abstraction
│       ├── sandbox.rs             # .aapp loader + package manager
│       ├── z3_solver.rs           # Z3 SMT solver (feature: z3-solver)
│       ├── sandbox_wasm.rs        # WebAssembly sandbox (feature: wasm-sandbox)
│       ├── uri.rs                 # Unified Robotics Interface (feature: uri)
│       ├── controller.rs          # Control loops & safety monitors (feature: uri)
│       ├── embodiment.rs          # Embodiment VLA pipeline (feature: embodiment)
│       ├── grid.rs                # Decentralized Cognitive Grids
│       ├── orchestrator.rs        # Sub-Agent Orchestration DSL runner (loops, parallel steps)
│       ├── marketplace.rs         # Agent Marketplace: catalog, search, templates, build, recommend
│       ├── governance.rs          # Governance Engine: contracts, compliance, policies, audit
│       ├── audit_export.rs        # Structured audit export (JSONL/CSV to stdout/file/S3)
│       ├── telemetry.rs           # OpenTelemetry OTLP tracing init
│       └── api.rs                 # axum REST API handlers (60+ endpoints)
│
├── agos-sign/                     # .aapp signing CLI tool (ED25519)
│   ├── Cargo.toml
│   └── src/main.rs
│
├── agos-demo/                     # Java Spring Boot demo UI (port 8080)
│   ├── pom.xml
│   └── src/main/java/com/agosdemo/
│       ├── AgosDemoApplication.java
│       ├── config/AppConfig.java
│       ├── service/
│       │   ├── AgosKernelClient.java    # REST client → Rust kernel
│       │   └── DemoOrchestrator.java    # 28-step demo scenario
│       ├── controller/WebController.java
│       └── resources/templates/demo.html  # Live-updating dashboard
│
├── demo/                          # Python prototype (legacy)
│   ├── demo.py
│   └── financial_auditor/
│
└── README.md
```

---

## Quick Start

### Prerequisites

- **Rust** 1.75+ (`rustc`, `cargo`)
- **Java** 17+ (`java`, `mvn`)
- **curl** (for API testing)

### 1. Start the Kernel

```bash
cd agos-kernel
cargo run
```

The kernel starts an HTTP REST API on `http://127.0.0.1:7400`.

```bash
curl http://127.0.0.1:7400/health
# {"status":"ok","system":"agOS Kernel v0.1.0"}
```

### 2. Start the Demo UI

```bash
cd agos-demo
mvn spring-boot:run
```

Open `http://127.0.0.1:8080` in a browser and click **Run Full Demo**.

### 3. Run Headless

```bash
curl -X POST http://127.0.0.1:8080/api/demo/run
curl http://127.0.0.1:8080/api/demo/status
```

---

## REST API Reference

The Rust kernel exposes REST endpoints. All use `Content-Type: application/json`.

### Feature Flags

Some endpoints require feature flags:

| Feature | Cargo flag | Additional endpoints |
|---------|-----------|---------------------|
| `z3-solver` | `--features z3-solver` | Enhances `/security/verify` with Z3 SMT |
| `wasm-sandbox` | `--features wasm-sandbox` | `POST /sandbox/execute` |
| `embodiment` | `--features embodiment` | 8 `/embodiment/*` endpoints |
| `uri` | `--features uri` | 28 `/uri/*` and `/controller/*` endpoints |
| `grid` | _always on_ | 7 `/grid/*` endpoints, no feature flag |
| OpenTelemetry | _always on_ | No additional endpoints; adds `#[instrument]` spans |
| Sub-Agent Orchestration | _always on_ | `POST /workflow/submit`, `POST /workflow/execute`, `GET /workflow/list`, `GET /workflow/:id` |
| Structured Audit Export | _always on_ | `POST /security/audit/export` |
| Marketplace | _always on_ | 7 `/marketplace/*` endpoints (list, search, get, scan, templates, build, recommend) |
| Governance | _always on_ | 7 `/governance/*` endpoints (contract CRUD, compliance, audit, policies) |
| Scheduler Policies | _always on_ | `GET /scheduler/policy`, `POST /scheduler/policy` |
| `.aapp` Signing | _always on_ | No additional endpoints; `AGOS_TRUSTED_KEY` env var |

Run with all features:
```bash
cargo run --features "z3-solver,wasm-sandbox,embodiment,uri"
```

### Kernel

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `POST` | `/kernel/spawn` | Spawn a cognitive process |
| `POST` | `/kernel/kill` | Kill a process |
| `GET` | `/kernel/processes` | List all processes |
| `GET` | `/kernel/process/{pid}` | Get process details |
| `POST` | `/kernel/panic` | Trigger kernel panic |

### Memory

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/memory/l1/write` | Write to L1 active context |
| `GET` | `/memory/l1/{pid}` | Read L1 context |
| `POST` | `/memory/l2/compress` | Compress L1 → L2 graph cache |
| `POST` | `/memory/vector` | Store vector embedding |
| `POST` | `/memory/page` | Cosine-similarity page-in |
| `GET` | `/memory/stats` | Memory statistics |
| `GET` | `/memory/episodic` | Episodic ledger |

### cIPC

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/cipc/route` | Route a structured message |
| `GET` | `/cipc/history` | Message history |

### Scheduler

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/scheduler/policy` | Get current scheduling policy |
| `POST` | `/scheduler/policy` | Change scheduling policy |

### Security

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/security/verify` | Ring 0 SMT verification |
| `GET` | `/security/audit` | Audit log |
| `POST` | `/security/audit/export` | Export audit log (JSONL/CSV to stdout/file/S3) |

### UMI

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/umi/reason` | Execute SYS_REASON |
| `POST` | `/umi/embed` | Execute SYS_EMBED |
| `POST` | `/umi/compress` | Execute SYS_COMPRESS |
| `POST` | `/umi/switch` | Hot-swap active driver |
| `GET` | `/umi/metrics` | Driver metrics |

### Sandbox

| Method | Path | Feature | Description |
|--------|------|---------|-------------|
| `POST` | `/sandbox/install` | — | Install `.aapp` package |
| `POST` | `/sandbox/launch` | — | Launch installed app |
| `GET` | `/sandbox/apps` | — | List available apps |
| `POST` | `/sandbox/execute` | `wasm-sandbox` | Execute WASM tool |

### Workflow Orchestration

| Method | Path | Feature | Description |
|--------|------|---------|-------------|
| `POST` | `/workflow/submit` | — | Submit a workflow definition (without executing) |
| `POST` | `/workflow/execute` | — | Execute a submitted workflow by ID |
| `GET` | `/workflow/list` | — | List all workflow instances |
| `GET` | `/workflow/{id}` | — | Get workflow instance details (status, current step, results) |

### Marketplace

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/marketplace/list` | List all agents in the marketplace catalog |
| `POST` | `/marketplace/search` | Search agents by query, capability, priority, or workflow flag |
| `GET` | `/marketplace/agent/{name}` | Get a single agent listing by name |
| `POST` | `/marketplace/scan` | Scan local `aapps/` directory and refresh catalog |
| `GET` | `/marketplace/templates` | List built-in agent templates (data_analyzer, report_generator, validator) |
| `POST` | `/marketplace/build` | Build a new `.aapp` agent from a template or custom spec |
| `POST` | `/marketplace/recommend` | Recommend a goal→workflow using UMI reasoning |

### Governance

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/governance/contract/create` | Create a governance contract with rules, limits, and success criteria |
| `GET` | `/governance/contract/{id}` | Get a governance contract by ID |
| `GET` | `/governance/contracts` | List all governance contracts |
| `POST` | `/governance/contract/{id}/remove` | Remove a governance contract |
| `GET` | `/governance/compliance` | Get compliance summary (total checks, violations, top violators) |
| `GET` | `/governance/audit` | Get compliance audit log |
| `POST` | `/governance/policy/create` | Create a policy bundling multiple contracts |
| `GET` | `/governance/policies` | List all governance policies |

### Embodiment

| Method | Path | Feature | Description |
|--------|------|---------|-------------|
| `POST` | `/embodiment/goal` | `embodiment` | Submit a goal with sensory inputs |
| `GET` | `/embodiment/goals` | `embodiment` | List all goals |
| `GET` | `/embodiment/goal/{id}` | `embodiment` | Get goal status |
| `GET` | `/embodiment/goal/{id}/plan` | `embodiment` | Get action plan |
| `POST` | `/embodiment/goal/{id}/clarify` | `embodiment` | Provide clarification |
| `POST` | `/embodiment/goal/{id}/execute` | `embodiment` | Execute plan |
| `GET` | `/embodiment/sensors` | `embodiment` | List available sensors |
| `GET` | `/embodiment/apps` | `embodiment` | List available agent apps |

### Grid

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/grid/join` | Register peer node on the grid |
| `GET` | `/grid/peers` | List known peers |
| `GET` | `/grid/stats` | Grid statistics |
| `POST` | `/grid/heartbeat` | Record heartbeat from peer |
| `POST` | `/grid/migrate` | Migrate agent to another node |
| `POST` | `/grid/receive` | Accept migrated agent |
| `POST` | `/grid/cipc/forward` | Forward cIPC message from another node |

### URI — Unified Robotics Interface (feature `uri`)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/uri/sense` | Read a sensor via a hardware driver |
| `POST` | `/uri/actuate` | Write to an actuator via a hardware driver |
| `POST` | `/uri/transform` | Apply a data transform pipeline |
| `POST` | `/uri/status` | Query driver status |
| `POST` | `/uri/switch` | Switch between hardware backends |
| `GET` | `/uri/metrics` | Hardware driver metrics |
| `GET` | `/uri/drivers` | List registered hardware drivers |
| `GET` | `/uri/drivers/{name}` | Get driver metadata |
| `POST` | `/uri/drivers/register` | Register a runtime hardware driver |
| `POST` | `/uri/drivers/unregister` | Unregister a runtime hardware driver |

### Controller — Hardware Control Loops (feature `uri`)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/controller/pid` | Create a PID controller |
| `GET` | `/controller/pid` | List PID controllers |
| `POST` | `/controller/pid/setpoint` | Update PID setpoint |
| `POST` | `/controller/pid/step` | Execute one PID control step |
| `POST` | `/controller/trajectory` | Create a trajectory plan |
| `GET` | `/controller/trajectory` | List trajectory plans |
| `POST` | `/controller/trajectory/start` | Start trajectory execution |
| `POST` | `/controller/trajectory/step` | Advance trajectory one waypoint |
| `POST` | `/controller/safety` | Create a safety monitor |
| `POST` | `/controller/safety/check` | Run safety check |
| `POST` | `/controller/safety/estop` | Trigger emergency stop |
| `POST` | `/controller/safety/estop/clear` | Clear emergency stop |
| `GET` | `/controller/safety/monitors` | List safety monitors |
| `GET` | `/controller/safety/events/{device_id}` | Get safety events for a device |
| `POST` | `/controller/transform` | Add a coordinate transform |
| `GET` | `/controller/transform` | List coordinate transforms |
| `GET` | `/controller/stats` | Controller statistics |

---

## The `.aapp` Package Format

Applications in agOS are declarative blueprints, not compiled binaries.

```
[App Bundle (.aapp)]
 ├── manifest.json          # Name, version, permissions, token quotas
 ├── logic_graphs.json      # State machine for reasoning execution
 ├── tools/                 # Declarative tool schemas
 └── validation_tests/      # Deterministic output assertions
```

Example `manifest.json`:

```json
{
  "name": "Financial Auditor",
  "version": "1.0.0",
  "permissions": ["financial:read", "financial:validate", "cipc:communicate"],
  "limits": { "max_tokens": 5000, "max_subagents": 3 },
  "priority": 2,
  "goal_metric": 0.95
}
```

---

## Demo Scenario: Financial Auditor + Accountant

The demo walks through all core agOS capabilities in steps across 13 phases:

| Phase | Steps | What It Shows |
|-------|-------|---------------|
| **P1** | 3 | Kernel boot, UMI driver registration, active driver |
| **P2** | 2 | `.aapp` installation with Ring 0 permission checks |
| **P3** | 3 | Process spawning (System 1/2), scheduler verification |
| **P4** | 5 | `SYS_REASON`, L1 write, L2 compression, vector embedding, cosine paging |
| **P5** | 3 | cIPC message routing, structured semantic payloads, history verification |
| **UMI** | 3 | Atomic hot-swap to Cloud driver, deep reasoning, restore |
| **P6** | 4 | Ring 0 ALLOW/DENY, SMT constraint enforcement, kernel panic, L1 flush |
| **P7** | 2 | Episodic ledger audit trail, Ring 0 audit log |
| **P8** | 3 | Memory stats, UMI metrics, process listing |
| **P9** | 17 | Robotic agents (Waiter/Chef/Supervisor), URI sensors, actuators, PID loops, trajectory planning, safety monitors, E-Stop |
| **P10** | 4 | Workflow orchestration with RepeatUntil loop step and 3-agent DAG |
| **P11** | — | Marketplace: agent catalog, search, templates, build, goal→workflow recommendation |
| **P12** | — | Governance: contracts, compliance checking, policies, audit log |

---

## Roadmap

- **Phase 1 ✓** — Deterministic Firmware Scaffolding (Rust kernel core, scheduler)
- **Phase 2 ✓** — Unified Model Interface (driver abstraction, local/cloud, hot-swap)
- **Phase 3 ✓** — Hierarchical Cognitive Memory + cIPC Bus (L1/L2/Vector RAM, structured IPC)
- **Phase 4 ✓** — Ring 0 Neuro-Symbolic Guard Layer (SMT solver, policy enforcement)
- **Phase 5 ✓** — Sandbox Installer + Integration Smoke Test (`.aapp` packages, demo scenario)
- **Phase 6 ✓** — Episodic Compression Daemon (background tokio task, configurable TTL, L2 compression)
- **Phase 7 ✓** — Native Z3 SMT Solver (optional feature, integrates Z3 into SecurityEngine)
- **Phase 8 ✓** — WebAssembly Sandbox (optional feature, wasmtime-based tool execution with fuel metering)
- **Phase 9 ✓** — Embodiment Layer (VLA pipeline, multimodal goal processing, sub-agent orchestration, figure-of-speech detection, UMI-powered semantic resolution)
- **Phase 10 ✓** — Decentralized Cognitive Grids (grid discovery, peer heartbeats, agent migration, remote cIPC forwarding)
- **Phase 11 ✓** — Unified Robotics Interface + Hardware Control (URI abstraction, hardware drivers, PID control, trajectory planning, safety monitors, coordinate transforms)
- **Phase 12 ✓** — Agent Marketplace (agent catalog, search, scan, built-in templates, agent building, goal→workflow recommendation via UMI)
- **Phase 13 ✓** — Governance Engine (governance contracts with rules/limits/criteria, compliance checking, policies, audit logging)

| Feature | Status |
|---------|--------|
| Phase 10 — Decentralized Cognitive Grids | ✓ Complete |
| Phase 11 — Unified Robotics Interface + Hardware Control | ✓ Complete |
| Phase 12 — Agent Marketplace | ✓ Complete |
| Phase 13 — Governance Engine | ✓ Complete |
| OpenTelemetry Instrumentation | ✓ Complete |
| Kubernetes Operator | ✓ Complete |
| Pluggable Scheduler Policies | ✓ Complete |
| Sub-Agent Orchestration DSL (loops, parallel, submit/execute API) | ✓ Complete |
| Secure .aapp Signing | ✓ Complete |
| Structured Audit Export | ✓ Complete |
| Persistent Storage (SQLite) | Planned |
| Rate Limiting and QoS | Planned |
| Token Accounting with Real Models | Planned |

---

## Guides

| Guide | Audience | Covers |
|-------|----------|--------|
| [`AGENT_DEV_GUIDE.md`](AGENT_DEV_GUIDE.md) | Agent developers | Building `.aapp` packages, state machines, tool schemas, deploying, cIPC, agent building from templates, goal→workflow recommendation |
| [`UMI_DRIVER_GUIDE.md`](UMI_DRIVER_GUIDE.md) | Model/driver developers | UMI architecture, `DriverState` struct, implementing syscalls, hot-swap mechanism, adding custom drivers |
| [`URI_DRIVER_GUIDE.md`](URI_DRIVER_GUIDE.md) | Hardware/driver developers | URI architecture, `HardwareDriver` struct, built-in vs runtime drivers, GPIO walkthrough |
| [`SYS_DEV_GUIDE.md`](SYS_DEV_GUIDE.md) | Kernel/system developers | Internal architecture, module deep-dives (marketplace, governance, orchestrator loops), concurrency model, adding subsystems & endpoints, build system, testing strategy |
| [`FEATURES_GUIDE.md`](FEATURES_GUIDE.md) | All contributors | Implementation blueprints for Phases 6–13 and all planned enhancements, with file-level change plans, effort estimates, and prioritization |

---

## Contributing

We welcome contributions from architects, engineers, and researchers working on neuro-symbolic systems, cognitive computing, and agent orchestration.

### Getting Started

#### Docker (Recommended)

```bash
docker compose up --build -d
curl http://localhost:7400/health
```

Full API docs: [API_REFERENCE.md](API_REFERENCE.md)  
Deployment guide: [SYS_DEV_GUIDE.md — Deployment](SYS_DEV_GUIDE.md#deployment)

#### Bare Metal / VM

```bash
cd agos-kernel
cargo build --release --features "z3-solver,wasm-sandbox,embodiment"
RUST_LOG=info ./target/release/agos-kernel
```

See [SYS_DEV_GUIDE.md](SYS_DEV_GUIDE.md) for systemd service setup, VM provisioning, and configuration.

1. **Fork** the repository.
2. **Clone** your fork:
   ```bash
   git clone https://github.com/<your-username>/agos.git
   ```
3. **Set up the development environment**:
   ```bash
   # Rust kernel
   cd agos-kernel
   cargo build
   cargo test
   
   # Java demo
   cd agos-demo
   mvn compile
   ```

### Development Workflow

1. **Pick an issue** from the tracker or propose a new one.
2. **Create a feature branch**:
   ```bash
   git checkout -b feat/my-feature
   ```
3. **Write code** following the conventions below.
4. **Add or update tests** — every module must have a corresponding test harness.
5. **Run the full test suite**:
   ```bash
   cd agos-kernel && cargo test
   cd agos-demo && mvn test
   ```
6. **Run the integration smoke test**:
   ```bash
   # Terminal 1
   cd agos-kernel && cargo run
   
   # Terminal 2
   curl -X POST http://127.0.0.1:8080/api/demo/run
   curl http://127.0.0.1:8080/api/demo/status
   ```
7. **Submit a pull request** with a clear description of changes.

### Code Conventions

- **Rust**: Follow `rustfmt` and `clippy`. Use `cargo fmt` before committing. Prefer safe, idiomatic Rust; avoid `unsafe` unless absolutely necessary.
- **Java**: Follow standard Java conventions (PascalCase classes, camelCase methods, 4-space indents). Use `@Override` annotations.
- **Documentation**: Document all public APIs with doc comments. Keep README and specs in sync with the code.
- **Commit messages**: Use conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`).

### What Needs Help

| Area | Skill Needed | Complexity |
|------|-------------|------------|
| Embodiment VLA drivers (ROS2, robot arms) | Robotics, Rust | Hard |
| URI hardware drivers (GPIO, I2C, SPI, CAN bus) | Embedded, Rust | Hard |
| Additional UMI drivers (OpenAI, Anthropic, local GGUF) | Rust, API design | Medium |
| Governance policy UI / dashboard | Java, React/Thymeleaf | Medium |
| Marketplace remote registry (pull from remote) | Rust, HTTP | Medium |
| Kubernetes deployment manifests | DevOps, K8s | Easy |
| More demo scenarios (.aapp packages) | JSON, state machines | Easy |
| Benchmarking and performance profiling | Rust, profiling tools | Medium |

### Code of Conduct

This project adheres to the [Contributor Covenant](https://www.contributor-covenant.org/). By participating, you agree to maintain a respectful, inclusive, and harassment-free community.

### Questions?

Open a [Discussion](https://github.com/anomalyco/opencode/issues) or reach out to the maintainers.

---

## License

Open-source architecture framework. See `LICENSE` for details.

---

## Citation

If you reference agOS in academic work, cite this repository:

```bibtex
@software{agos2025,
  title = {agOS: Agentic Operating System},
  year = {2025},
  url = {https://github.com/anomalyco/opencode}
}
```
