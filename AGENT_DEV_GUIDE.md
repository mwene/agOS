# Agent Developer Guide

This guide covers how to create, package, deploy, and run cognitive agents on agOS. Agents are packaged as `.aapp` (Agentic Application) bundles — declarative blueprints that define permissions, reasoning state machines, tool schemas, and validation assertions.

---

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Anatomy of a .aapp Package](#anatomy-of-a-aapp-package)
- [manifest.json Reference](#manifestjson-reference)
- [logic_graphs.json — State Machines](#logic_graphsjson--state-machines)
- [tools/ — Declarative Tool Schemas](#tools--declarative-tool-schemas)
- [validation_tests/ — Deterministic Assertions](#validation_tests--deterministic-assertions)
- [Deploying an Agent](#deploying-an-agent)
- [Agent Lifecycle](#agent-lifecycle)
- [Inter-Agent Communication (cIPC)](#inter-agent-communication-cipc)
- [End-to-End Walkthrough: Building a Sentiment Analyzer Agent](#end-to-end-walkthrough-building-a-sentiment-analyzer-agent)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

---

## Architecture Overview

```
   ┌──────────────────────────────────────┐
   │          Your .aapp Agent             │
   │  ┌─────────┐  ┌────────┐  ┌───────┐ │
   │  │ Manifest │  │ State  │  │ Tools │ │
   │  │ .json    │  │ Machine│  │ (json)│ │
   │  └─────────┘  └────────┘  └───────┘ │
   └──────────────┬───────────────────────┘
                  │ 1. install + launch
                  v
   ┌──────────────────────────────────────┐
   │           agOS Kernel (Ring 0)        │
   │  ┌──────────┐  ┌────────┐  ┌──────┐ │
   │  │Scheduler │  │Memory  │  │cIPC  │ │
   │  │(token    │  │(L1/L2/ │  │Bus   │ │
   │  │ quotas)  │  │Vector) │  │      │ │
   │  └──────────┘  └────────┘  └──────┘ │
   │  ┌──────────┐  ┌──────────────────┐ │
   │  │Security  │  │UMI Driver Layer  │ │
   │  │(Ring 0   │  │(Local/Cloud/etc) │ │
   │  │ SMT)     │  │                  │ │
   │  └──────────┘  └──────────────────┘ │
   └──────────────────────────────────────┘
```

Agents do not contain model weights or compiled binaries. They are **declarative specifications** that the kernel interprets — the kernel provides scheduling, memory, security, and model inference through the UMI layer.

---

## Anatomy of a .aapp Package

A `.aapp` package is a directory (or tarball) with this structure:

```
your-agent/
├── manifest.json           # Required: identity, permissions, resource limits
├── logic_graphs.json       # Required: state machine definition
├── tools/                  # Optional: tool schemas the agent exposes
│   ├── tool_one.json
│   └── tool_two.json
└── validation_tests/       # Optional: deterministic output assertions
    ├── test_001.json
    └── test_002.json
```

### Minimal Example

```
hello-agent/
├── manifest.json
└── logic_graphs.json
```

---

## manifest.json Reference

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Human-readable agent name |
| `version` | string | yes | Semantic version |
| `description` | string | no | Purpose of the agent |
| `permissions` | array[string] | yes | Ring 0 permission scopes |
| `limits.max_tokens` | integer | yes | Token budget before forced termination |
| `limits.max_subagents` | integer | no | Max child processes this agent may spawn |
| `priority` | integer | yes | Scheduling priority (1 = System 1, 2+ = System 2) |
| `goal_metric` | float | no | Target performance threshold (0.0–1.0) |

### Permission Scopes

Permissions are strings in `domain:action` or `domain:*` format:

| Pattern | Effect |
|---------|--------|
| `financial:read` | Read financial data |
| `financial:*` | All financial operations |
| `cipc:communicate` | Send/receive cIPC messages |
| `memory:page` | Page in/out of Vector RAM |
| `umi:reason` | Invoke SYS_REASON |
| `*:*` | **Danger:** unrestricted access (Ring 0 may still block blocked_actions) |

**Default SMT constraints** always block these actions regardless of permissions:
- `network:write`
- `filesystem:write`
- `sandbox:escape`
- Any action with `capability` = `admin:escalate`

### Example

```json
{
  "name": "Financial Auditor",
  "version": "1.0.0",
  "description": "Multi-agent financial validation and auditing suite",
  "permissions": ["financial:read", "financial:validate", "cipc:communicate", "memory:page"],
  "limits": { "max_tokens": 5000, "max_subagents": 3 },
  "priority": 2,
  "goal_metric": 0.95
}
```

---

## logic_graphs.json — State Machines

Agents execute as **deterministic state machines**. The kernel tracks the current state and transitions based on external triggers or cIPC messages.

### Schema

```json
{
  "initial_state": "start",
  "states": {
    "start": {
      "transitions": {
        "event_name": "next_state",
        "error": "error_state"
      }
    },
    "next_state": {
      "transitions": {
        "finish": "__terminal__"
      }
    }
  }
}
```

- **`initial_state`** (string, required): The state the agent starts in when spawned.
- **`states`** (map, required): Each state has a `transitions` map. Transition keys are event names; values are target states.
- **`__terminal__`**: A reserved target state. Agents in this state are considered complete.

### Example: 5-State Financial Auditor

```json
{
  "initial_state": "validate_balance",
  "states": {
    "validate_balance": {
      "transitions": {
        "balance_ok": "verify_transactions",
        "balance_error": "flag_anomaly"
      }
    },
    "verify_transactions": {
      "transitions": {
        "all_verified": "generate_report",
        "suspicious": "flag_anomaly"
      }
    },
    "flag_anomaly": {
      "transitions": {
        "resolved": "verify_transactions",
        "escalated": "escalate_to_auditor"
      }
    },
    "escalate_to_auditor": {
      "transitions": {
        "audited": "generate_report"
      }
    },
    "generate_report": {
      "transitions": {
        "complete": "__terminal__"
      }
    }
  }
}
```

---

## tools/ — Declarative Tool Schemas

Tool schemas define the **input contract** the agent expects. They are JSON objects following a lightweight schema convention.

### Schema

```json
{
  "name": "tool_name",
  "description": "What this tool does",
  "input_schema": {
    "type": "object",
    "required": ["field_a", "field_b"],
    "properties": {
      "field_a": { "type": "array", "items": { "type": "number" } },
      "field_b": { "type": "string" }
    }
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Tool identifier |
| `description` | string | yes | Human-readable purpose |
| `input_schema.type` | string | yes | Must be `"object"` |
| `input_schema.required` | array | no | Fields that must be present |
| `input_schema.properties` | map | no | Per-field type definitions |

### Example: reconcile.json

```json
{
  "name": "reconcile_accounts",
  "description": "Reconcile two financial ledgers and return discrepancies",
  "input_schema": {
    "type": "object",
    "required": ["ledger_a", "ledger_b"],
    "properties": {
      "ledger_a": { "type": "array", "items": { "type": "number" } },
      "ledger_b": { "type": "array", "items": { "type": "number" } }
    }
  }
}
```

---

## validation_tests/ — Deterministic Assertions

Validation tests are JSON assertions that the kernel can run to verify agent correctness before deployment.

### Schema

```json
{
  "test_id": "test-name-001",
  "assertions": [
    { "field": "field_name", "operator": "exists" },
    { "field": "payload", "operator": "is_dict" }
  ]
}
```

Supported operators:
- `exists` — the field must be present
- `is_dict` — the field must be a JSON object
- `is_list` — the field must be a JSON array

### Example

```json
{"test_id": "schema-compliance-001", "assertions": [{"field": "type", "operator": "exists"}, {"field": "payload", "operator": "is_dict"}]}
```

---

## Deploying an Agent

### Prerequisites

- An agOS kernel running at `http://127.0.0.1:7400`
- Your `.aapp` package directory on the same filesystem as the kernel

### Step 1: Install the .aapp Package

Install by POSTing the package name to the sandbox endpoint. The kernel resolves the path relative to its working directory.

```bash
curl -X POST http://127.0.0.1:7400/sandbox/install \
  -H 'Content-Type: application/json' \
  -d '{"name": "your-agent", "aapps_path": "aapps"}'
```

**Response:**

```json
{
  "status": "installed",
  "name": "Your Agent",
  "version": "1.0.0",
  "permissions": ["financial:read", "financial:validate", "cipc:communicate", "memory:page"],
  "token_quota": 5000,
  "states": 5
}
```

If any permission is denied by Ring 0 SMT constraints, the install fails with an error message.

### Step 2: Launch the Agent

Launching creates a new cognitive process from the agent's manifest and registers it with the scheduler.

```bash
curl -X POST http://127.0.0.1:7400/sandbox/launch \
  -H 'Content-Type: application/json' \
  -d '{"name": "your-agent", "parent_pid": null}'
```

**Response:**

```json
{
  "pid": "proc-a1b2c3d4e5f6",
  "priority": 2,
  "token_quota": 5000
}
```

The returned `pid` is used for all subsequent interactions (memory, cIPC, lifecycle).

### Alternative: Spawn Programmatically

Instead of installing from a package, you can spawn a process by passing a JSON manifest directly:

```bash
curl -X POST http://127.0.0.1:7400/kernel/spawn \
  -H 'Content-Type: application/json' \
  -d '{"manifest": {"name": "Custom Agent", "priority": 2, "limits": {"max_tokens": 5000}, "permissions": ["financial:read"]}, "parent_pid": null}'
```

---

## Agent Lifecycle

```
  ┌──────────┐
  │  SPAWN   │  (kernel/spawn or sandbox/launch)
  └────┬─────┘
       │
       v
  ┌──────────┐     ┌─────────────┐     ┌──────────────┐
  │ RUNNING  │────>│   PANIC     │────>│  TERMINATED  │
  │ (token   │     │ (violation) │     │ (L1 flushed) │
  │  quota)  │     └─────────────┘     └──────────────┘
  └────┬─────┘
       │
       v
  ┌──────────┐
  │ COMPLETE │  (state machine reaches __terminal__)
  │ IDLE     │
  └──────────┘
```

1. **SPAWN**: Agent registered in scheduler, gets a PID and token quota.
2. **RUNNING**: Agent can reason (UMI), write memory (L1), communicate (cIPC).
3. **Token Exhaustion**: If `tokens_used > max_tokens`, the process is terminated.
4. **PANIC**: A security violation triggers `kernel_panic`. L1 is flushed, alert broadcast, process unregistered.
5. **KILL**: Explicit termination via `kernel/kill`.

### Check Running Processes

```bash
curl http://127.0.0.1:7400/kernel/processes
```

---

## Inter-Agent Communication (cIPC)

Agents communicate over the **Cognitive Inter-Process Communication (cIPC) bus**. Messages are structured JSON payloads with typed schemas.

### Sending a Message

```bash
curl -X POST http://127.0.0.1:7400/cipc/route \
  -H 'Content-Type: application/json' \
  -d '{
    "from": "proc-sender-pid",
    "to": "proc-receiver-pid",
    "payload": {
      "type": "financial.verification_request",
      "action": "cross_validate",
      "balance_total": 24765.49
    }
  }'
```

### Viewing Message History

```bash
curl http://127.0.0.1:7400/cipc/history
```

### Message Schema

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Auto-generated UUID |
| `from` | string | Sender PID |
| `to` | string | Recipient PID |
| `payload` | object | Application-specific JSON payload |
| `timestamp` | string | RFC 3339 timestamp |

### Route Permissions

By default, any process can route to any other. You can restrict routes by calling `register_process` on the cIPC bus with an explicit allowlist. If an allowlist is set, messages to non-listed PIDs are rejected.

---

## End-to-End Walkthrough: Building a Sentiment Analyzer Agent

This walkthrough creates a `sentiment_analyzer` agent from scratch, deploys it, and runs it.

### Step 1: Create the Package

```bash
mkdir -p aapps/sentiment_analyzer/{tools,validation_tests}
```

### Step 2: Write manifest.json

```json
{
  "name": "Sentiment Analyzer",
  "version": "1.0.0",
  "description": "Analyzes text sentiment and returns scores",
  "permissions": ["umi:reason", "memory:page"],
  "limits": { "max_tokens": 2000, "max_subagents": 1 },
  "priority": 1,
  "goal_metric": 0.9
}
```

### Step 3: Write logic_graphs.json

```json
{
  "initial_state": "analyze",
  "states": {
    "analyze": {
      "transitions": {
        "positive": "report",
        "negative": "report",
        "neutral": "report"
      }
    },
    "report": {
      "transitions": {
        "done": "__terminal__"
      }
    }
  }
}
```

### Step 4: (Optional) Add a Tool Schema

`tools/analyze.json`:

```json
{
  "name": "score_sentiment",
  "description": "Score sentiment of a text passage",
  "input_schema": {
    "type": "object",
    "required": ["text"],
    "properties": {
      "text": { "type": "string" }
    }
  }
}
```

### Step 5: Deploy and Run

```bash
# Start the kernel
cd agos-kernel && cargo run &

# Install
curl -X POST http://127.0.0.1:7400/sandbox/install \
  -H 'Content-Type: application/json' \
  -d '{"name": "sentiment_analyzer", "aapps_path": "aapps"}'

# Launch
LAUNCH=$(curl -s -X POST http://127.0.0.1:7400/sandbox/launch \
  -H 'Content-Type: application/json' \
  -d '{"name": "sentiment_analyzer"}')
PID=$(echo $LAUNCH | python3 -c "import sys,json; print(json.load(sys.stdin)['pid'])")

# Do something with the agent
curl -X POST http://127.0.0.1:7400/umi/reason \
  -H 'Content-Type: application/json' \
  -d '{"prompt_vector": ["analyze_sentiment", "positive_review", "customer_feedback"]}'

# Check up on it
curl http://127.0.0.1:7400/kernel/processes
```

---

## Best Practices

1. **Permission Minimalism** — Request only the permissions your agent needs. `financial:*` is safer than `*:*`.
2. **State Machine Design** — Keep state machines flat (< 10 states). Deeply nested machines are harder to debug.
3. **Token Budgeting** — Set `max_tokens` based on the complexity of your agent. Each UMI reason call costs 2–5 tokens per prompt element.
4. **Naming** — Use `kebab-case` for directory names, `lowercase_with_underscores` for state names.
5. **Validation Tests** — Always include at least one validation test. It serves as documentation for your agent's expected behavior.
6. **Parent-Child Relationships** — When spawning sub-agents, pass the `parent_pid` to establish a hierarchical relationship visible in the episodic ledger.
7. **Error States** — Include an `error` transition in every state to handle failures gracefully.

---

## Troubleshooting

### "Permission 'X' denied by Ring 0"

The action is blocked by SMT constraints. Check:
- Is it one of the blocked actions (`network:write`, `filesystem:write`, `sandbox:escape`)?
- Does the action's capability field contain `admin:escalate`?
- Does your `permissions` list include the required pattern?

### "Token quota exhausted"

The agent has exceeded its `max_tokens` budget. Either:
- Increase `limits.max_tokens` in the manifest.
- Reduce the number of UMI calls.
- Kill and re-spawn the process.

### "Could not read manifest.json"

The kernel resolves paths relative to its working directory. Ensure:
- The directory exists at `{aapps_path}/{agent_name}/manifest.json`.
- The JSON is valid.

### "Process not found"

The PID may have been terminated (panic, quota exhaustion, or explicit kill). Use `GET /kernel/processes` to list active processes.

### cIPC route failures

- Verify both PIDs are live (`GET /kernel/processes`).
- Check that the recipient has `cipc:communicate` permission.
- If route permissions are configured, ensure the sender is on the recipient's allowlist.

---

## Reference: All REST Endpoints for Agents

| Method | Endpoint | Use |
|--------|----------|-----|
| `POST` | `/sandbox/install` | Install a .aapp package |
| `POST` | `/sandbox/launch` | Launch an installed app as a process |
| `GET` | `/sandbox/apps` | List installable apps |
| `POST` | `/kernel/spawn` | Spawn a process from inline manifest |
| `POST` | `/kernel/kill` | Kill a process |
| `GET` | `/kernel/process/{pid}` | Get process details |
| `GET` | `/kernel/processes` | List all processes |
| `POST` | `/kernel/panic` | Trigger kernel panic for a process |
| `POST` | `/umi/reason` | Execute SYS_REASON |
| `POST` | `/umi/embed` | Generate embeddings |
| `POST` | `/memory/l1/write` | Write to L1 context |
| `GET` | `/memory/l1/{pid}` | Read L1 context |
| `POST` | `/cipc/route` | Route a message to another agent |
| `GET` | `/cipc/history` | View message history |
| `POST` | `/security/verify` | Check if an action is permitted |
| `GET` | `/memory/episodic` | View episodic ledger |
| `GET` | `/memory/stats` | Memory statistics |
