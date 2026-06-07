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
│       └── api.rs                 # axum REST API handlers (25 endpoints)
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

The Rust kernel exposes 25 REST endpoints. All use `Content-Type: application/json`.

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

### Security

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/security/verify` | Ring 0 SMT verification |
| `GET` | `/security/audit` | Audit log |

### UMI

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/umi/reason` | Execute SYS_REASON |
| `POST` | `/umi/embed` | Execute SYS_EMBED |
| `POST` | `/umi/compress` | Execute SYS_COMPRESS |
| `POST` | `/umi/switch` | Hot-swap active driver |
| `GET` | `/umi/metrics` | Driver metrics |

### Sandbox

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/sandbox/install` | Install `.aapp` package |
| `POST` | `/sandbox/launch` | Launch installed app |
| `GET` | `/sandbox/apps` | List available apps |

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

The pre-seed demo walks through all core agOS capabilities in 28 steps across 8 phases:

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

---

## Roadmap

- **Phase 1 ✓** — Deterministic Firmware Scaffolding (Rust kernel core, scheduler)
- **Phase 2 ✓** — Unified Model Interface (driver abstraction, local/cloud, hot-swap)
- **Phase 3 ✓** — Hierarchical Cognitive Memory + cIPC Bus (L1/L2/Vector RAM, structured IPC)
- **Phase 4 ✓** — Ring 0 Neuro-Symbolic Guard Layer (SMT solver, policy enforcement)
- **Phase 5 ✓** — Sandbox Installer + Integration Smoke Test (`.aapp` packages, demo scenario)

**Future:** See [`FEATURES_GUIDE.md`](FEATURES_GUIDE.md) for detailed implementation blueprints, file-level change plans, and testing strategies for all upcoming features.

| Feature | Effort | Impact |
|---------|--------|--------|
| Phase 6 — Episodic Compression Daemon | 3 days | Medium |
| Phase 7 — Native Z3 SMT Solver | 5 days | Medium |
| Phase 8 — WebAssembly Sandbox | 2 weeks | High |
| Phase 9 — Embodiment Layer (VLA pipeline) | 3 weeks | High |
| Phase 10 — Decentralized Cognitive Grids | 4 weeks | High |
| Persistent Storage (SQLite) | 2 days | High |
| Pluggable Scheduler Policies | 3 days | Medium |
| Sub-Agent Orchestration DSL | 5 days | High |
| Secure .aapp Signing | 2 days | Medium |
| Rate Limiting and QoS | 1 day | Medium |
| OpenTelemetry Instrumentation | 2 days | Low |
| Kubernetes Operator | 3 days | Medium |
| Structured Audit Export | 2 days | Medium |
| Token Accounting with Real Models | 1 day | High |

---

## Guides

| Guide | Audience | Covers |
|-------|----------|--------|
| [`AGENT_DEV_GUIDE.md`](AGENT_DEV_GUIDE.md) | Agent developers | Building `.aapp` packages, state machines, tool schemas, deploying, cIPC, end-to-end walkthrough |
| [`UMI_DRIVER_GUIDE.md`](UMI_DRIVER_GUIDE.md) | Model/driver developers | UMI architecture, `DriverState` struct, implementing syscalls, hot-swap mechanism, adding custom drivers |
| [`SYS_DEV_GUIDE.md`](SYS_DEV_GUIDE.md) | Kernel/system developers | Internal architecture, module deep-dives, concurrency model, adding subsystems & endpoints, build system, testing strategy |
| [`FEATURES_GUIDE.md`](FEATURES_GUIDE.md) | All contributors | Implementation blueprints for Phases 6–10 and all planned enhancements, with file-level change plans, effort estimates, and prioritization |

---

## Contributing

We welcome contributions from architects, engineers, and researchers working on neuro-symbolic systems, cognitive computing, and agent orchestration.

### Getting Started

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
| SMT solver integration (Z3) | Formal methods, Rust | Hard |
| WebAssembly tool sandbox | WASM, Rust, security | Hard |
| Episodic compression daemon | Systems programming, Rust | Medium |
| Additional UMI drivers (OpenAI, Anthropic, local GGUF) | Rust, API design | Medium |
| `.aapp` marketplace API | Java, REST, Spring Boot | Medium |
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
