# agOS HTTP API Reference

Base URL: `http://<host>:7400`

All requests and responses use `Content-Type: application/json`.

---

## Table of Contents

- [Authentication](#authentication)
- [Health](#health)
- [Kernel](#kernel)
- [Memory](#memory)
- [cIPC](#cipc)
- [Security](#security)
- [UMI — Unified Model Interface](#umi--unified-model-interface)
- [Driver Management](#driver-management)
- [Sandbox](#sandbox)
- [Scheduler](#scheduler)
- [WASM Sandbox](#wasm-sandbox-feature-wasm-sandbox)
- [Workflow Orchestration](#workflow-orchestration)
- [Embodiment](#embodiment-feature-embodiment)
- [URI — Unified Robotics Interface](#uri--unified-robotics-interface-feature-uri)
- [Controller — Hardware Control Loops](#controller--hardware-control-loops-feature-uri)
- [Grid — Decentralised Cognitive Grids](#grid--decentralised-cognitive-grids)

---

## Authentication

Authentication is **optional** and **disabled by default**. When enabled via `POST /security/auth`, all endpoints except `/health` and `/security/auth*` require the `X-API-Key` header.

### `POST /security/auth` — Enable or disable API key auth

```json
{"api_key": "my-secret-key"}    // enable
{"api_key": ""}                 // disable
```

Response (enabled):
```json
{"ok": true, "message": "API key authentication enabled. ..."}
```

Response (disabled):
```json
{"ok": true, "message": "API key authentication disabled."}
```

### `POST /security/auth/agent` — Set agent token

```json
{"agent_id": "agent-1", "token": "tok-abc"}
```

Response:
```json
{"ok": true, "agent_id": "agent-1"}
```

---

## Health

### `GET /health`

Alias for system liveness.

Response:
```json
{"status": "ok", "system": "agOS Kernel v0.1.0"}
```

---

## Kernel

### `POST /kernel/spawn` — Spawn a new process

```json
{"manifest": {"name": "my-agent", ...}, "parent_pid": null}
```

Response:
```json
{"pid": "proc-abc123", "priority": 1, "token_quota": 1000, "goal_metric": 0.0}
```

### `POST /kernel/kill` — Kill a process

```json
{"pid": "proc-abc123"}
```

Response:
```json
{"ok": true, "pid": "proc-abc123"}
```

### `GET /kernel/processes` — List all processes

Response:
```json
{"processes": [{"pid": "proc-abc123", ...}, ...]}
```

### `GET /kernel/process/:pid` — Get process details

Response:
```json
{"process": {"pid": "proc-abc123", "priority": 1, ...}}
```

Error:
```json
{"error": "Process not found", "pid": "nonexistent"}
```

### `POST /kernel/panic` — Trigger a kernel panic for a process

```json
{"pid": "proc-abc123", "reason": "Out of tokens"}
```

Response:
```json
{"panic": {"pid": "proc-abc123", ...}, "alert": "Broadcast sent for PID proc-abc123", "l1_flushed": true}
```

---

## Memory

### `POST /memory/l1/write` — Write to L1 (active context)

```json
{"pid": "proc-abc123", "data": [{"role": "user", "content": "..."}]}
```

Response:
```json
{"ok": true}
```

### `GET /memory/l1/:pid` — Read L1 context

Response:
```json
{"pid": "proc-abc123", "data": [...]}
```

### `POST /memory/l2/compress` — Compress L1 to L2

```json
{"pid": "proc-abc123"}
```

Response:
```json
{"pid": "proc-abc123", "compressed": {compressed_graph}}
```

### `POST /memory/vector` — Store a vector embedding

```json
{"vector": [0.1, 0.2, ...], "metadata": {"source": "..."}}
```

Response:
```json
{"ok": true}
```

### `POST /memory/page` — Page in memory by vector similarity

```json
{"query_vector": [0.1, 0.2, ...], "top_k": 3}
```

Response:
```json
{"pages": [...], "count": 3}
```

### `GET /memory/stats` — Memory system statistics

Response:
```json
{"stats": {"l1_count": ..., "l2_count": ..., "vector_count": ..., "episodic_count": ..., "compressed_count": ...}}
```

### `GET /memory/episodic` — Get episodic event log

Response:
```json
{"entries": [...], "count": ...}
```

### `POST /memory/episodic/compress` — Compress stale episodic entries

Response:
```json
{"ok": true, "compressed": 5}
```

---

## cIPC

### `POST /cipc/route` — Route a message between processes

```json
{"from": "proc-abc123", "to": "proc-def456", "payload": {"type": "request", "data": ...}}
```

Response:
```json
{"ok": true, "message_id": "msg-..."}
```

Error:
```json
{"ok": false, "error": "Process proc-def456 not found"}
```

### `GET /cipc/history` — Get message history

Response:
```json
{"messages": [...], "count": ...}
```

---

## Security

### `POST /security/verify` — Verify an action against Ring 0 policy

```json
{"action_vector": {"action": "read", "target": "memory", "resource": "proc-abc123"}, "permissions": ["kernel:*"]}
```

Response (allowed):
```json
{"allowed": true, "reason": "verified"}
```

Response (denied):
```json
{"allowed": false, "reason": "...denial reason..."}
```

### `GET /security/audit` — Get security audit log

Response:
```json
{"entries": [...], "count": ..., "allowed": ..., "denied": ...}
```

### `POST /security/audit/export` — Export audit log to external sink

```json
{"format": "jsonl", "since": "2026-06-01T00:00:00Z", "sink": {"type": "stdout"}}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `format` | string | yes | `"jsonl"` or `"csv"` |
| `since` | string | no | ISO 8601 timestamp; only entries after this time are included |
| `sink.type` | string | yes | `"stdout"`, `"jsonl_file"`, or `"s3"` |
| `sink.path` | string | for `jsonl_file` | Local file path to write to |
| `sink.bucket` | string | for `s3` | S3 bucket name |
| `sink.key` | string | for `s3` | S3 object key |
| `sink.endpoint` | string | for `s3` | S3-compatible endpoint URL |
| `sink.region` | string | for `s3` | AWS region (default: `us-east-1`) |

Response:
```json
{"ok": true, "exported": 12, "format": "jsonl", "sink": "stdout"}
```

Error:
```json
{"ok": false, "error": "Unsupported sink type: 'splunk'"}
```

---

## UMI — Unified Model Interface

### `POST /umi/reason` — Execute a reasoning pass

```json
{"prompt_vector": ["What is 2+2?"]}
```

Response:
```json
{"output": ["reasoned:What is 2+2?"], "tokens_used": 2, "model": "LocalEngine-v0", "mode": "SYS_REASON"}
```

### `POST /umi/embed` — Generate embeddings

```json
{"data": ["hello", "world"]}
```

Response:
```json
{"embedding": [0.42, 0.13, ...], "driver": "LocalEngine-v0"}
```

### `POST /umi/compress` — Compress data via the active driver

```json
{"data": ["entry1", "entry2", ...]}
```

Response:
```json
{"compressed": {"original_length": 2, "driver": "LocalEngine-v0", "compressed_at": "2026-..."}, "driver": "LocalEngine-v0"}
```

### `POST /umi/switch` — Switch active driver

```json
{"driver_name": "CloudGPT-HighReason"}
```

Response:
```json
{"ok": true, "active_driver": "CloudGPT-HighReason", "capability": 5}
```

Error:
```json
{"ok": false, "error": "Driver 'Foo' not found. ..."}
```

### `GET /umi/metrics` — Driver usage metrics

Response:
```json
{"metrics": [{"name": "LocalEngine-v0", "capability_level": 2, "calls": 5, "total_tokens": 10}, ...], "active_driver": "LocalEngine-v0"}
```

---

## Driver Management

### `GET /umi/drivers` — List all registered drivers

Response:
```json
{"count": 3, "drivers": [{"name": "LocalEngine-v0", "capability_level": 2, "driver_type": "builtin", ...}, ...]}
```

### `GET /umi/driver/info/:name` — Get driver metadata (no API keys)

Response:
```json
{"name": "GPT4", "capability_level": 5, "driver_type": "runtime", "token_cost": 10, "prefix": "gpt4", ...}
```

Error:
```json
{"error": "Driver 'Foo' not found"}
```

### `GET /umi/driver/download/:name` — Download full driver config (includes API keys)

Requires `X-API-Key` header if authentication is enabled.

Response:
```json
{"driver": {"name": "GPT4", ...}, "config": {"api_key": "sk-xxxx", "model": "gpt-4"}}
```

### `POST /umi/driver/register` — Register a new runtime driver

```json
{
  "name": "GPT4",
  "capability_level": 5,
  "token_cost": 10,
  "prefix": "gpt4",
  "embed_modulus": 100000,
  "max_output": 20,
  "config": {"api_key": "sk-xxxx", "model": "gpt-4"}
}
```

All fields except `name` and `capability_level` are optional with defaults:
- `token_cost` = 1
- `prefix` = `"inferred"`
- `embed_modulus` = 10000
- `max_output` = 10
- `config` = `{}`

Response:
```json
{"ok": true, "name": "GPT4"}
```

Error:
```json
{"error": "Driver 'GPT4' already registered"}
```

### `POST /umi/driver/config/:name` — Update driver config

```json
{"config": {"api_key": "sk-yyyy", "model": "gpt-4-turbo"}}
```

Response:
```json
{"ok": true, "name": "GPT4"}
```

### `POST /umi/driver/unregister` — Unregister a runtime driver

```json
{"name": "GPT4"}
```

Response:
```json
{"ok": true}
```

Error:
```json
{"error": "Cannot unregister built-in driver 'LocalEngine-v0'"}
```

---

## Sandbox

### `POST /sandbox/install` — Install an AApp package

```json
{"name": "financial_auditor", "aapps_path": "aapps"}
```

Response:
```json
{"status": "installed", "name": "financial_auditor", "version": "1.0.0", "permissions": [...], "token_quota": 1000, "states": 3}
```

### `POST /sandbox/launch` — Launch an installed AApp

```json
{"name": "financial_auditor", "parent_pid": null}
```

Response:
```json
{"pid": "proc-...", "priority": 1, "token_quota": 1000}
```

### `GET /sandbox/apps` — List installed apps

Response:
```json
{"apps": ["financial_auditor", "accountant"]}
```

---

## Scheduler

### `GET /scheduler/policy` — Get current scheduler policy

Response:
```json
{"policy": "priority_preemption"}
```

### `POST /scheduler/policy` — Change scheduler policy

```json
{"policy": "round_robin"}
```

Supported policies: `"priority_preemption"`, `"round_robin"`, `"fifo"`, `"token_aware"`.

Response:
```json
{"ok": true, "policy": "round_robin"}
```

Error:
```json
{"ok": false, "error": "Unknown policy: 'unknown'"}
```

---

## WASM Sandbox (feature `wasm-sandbox`)

### `POST /sandbox/execute` — Execute a WASM module

```json
{"wasm_base64": "AGFzb...", "input": {"action": "compute"}, "pid": "proc-abc123", "fuel_quota": 100000}
```

Response:
```json
{"ok": true, "result": "..."}
```

---

## Workflow Orchestration

### `POST /workflow/run` — Run a workflow from a .aapp package

```json
{"name": "financial_audit", "aapps_path": "aapps"}
```

Response:
```json
{"workflow_id": "wf-abc123", "status": "running", "current_step": "validate_balance"}
```

### `GET /workflow/status/:id` — Get workflow status

Response:
```json
{
  "workflow_id": "wf-abc123",
  "status": "running",
  "definition": {"steps": [...]},
  "current_step": "verify_transactions",
  "steps": {
    "validate_balance": {"status": "succeeded", "child_pid": "proc-...", "started_at": "...", "completed_at": "..."},
    "verify_transactions": {"status": "running", "child_pid": "proc-...", "started_at": "..."}
  },
  "created_at": "..."
}
```

Error:
```json
{"error": "Workflow not found", "workflow_id": "wf-nonexistent"}
```

---

## Embodiment (feature `embodiment`)

### `POST /embodiment/goal` — Submit a goal

```json
{
  "statement": "Navigate to the kitchen and prepare coffee",
  "sensory_inputs": [{"modality": "text", "data": "I see a door ahead"}]
}
```

Response:
```json
{"goal": {"id": "goal-...", "status": "Understanding", ...}, "plan": null}
```

### `GET /embodiment/goals` — List all goals

Response:
```json
{"goals": [...], "count": ...}
```

### `GET /embodiment/goal/:goal_id` — Get goal status

Response:
```json
{"goal": {"id": "goal-...", "status": "Executing", ...}}
```

### `GET /embodiment/goal/:goal_id/plan` — Get goal plan

Response:
```json
{"plan": [{"step": "move_to_kitchen", ...}, ...]}
```

### `POST /embodiment/goal/:goal_id/clarify` — Provide clarification

```json
{"clarification": "Use the back door, the front is locked"}
```

Response:
```json
{"ok": true, "plan": [...]}
```

### `POST /embodiment/goal/:goal_id/execute` — Execute the plan

Response:
```json
{"ok": true, "goal": {"id": "goal-...", "status": "Executing"}}
```

### `GET /embodiment/sensors` — List available sensors

Response:
```json
{"sensors": ["text", "image", "audio", "video", "haptic", "olfactory", "gustatory"]}
```

### `GET /embodiment/apps` — List agent apps (sub-agents)

Response:
```json
{"apps": ["navigator", "manipulator"]}
```

---

## URI — Unified Robotics Interface (feature `uri`)

The URI provides hardware abstraction — read sensors, write actuators, apply data transforms, and manage hardware drivers.

### `POST /uri/sense` — Read a sensor

```json
{"device_id": "temp_01", "driver_name": "SimulatedTemperature"}
```

`driver_name` is optional (uses active driver for device if omitted).

Response:
```json
{"device_id": "temp_01", "value": 23.5, "unit": "celsius", "driver": "SimulatedTemperature", "timestamp": "..."}
```

### `POST /uri/actuate` — Write to an actuator

```json
{"device_id": "servo_01", "value": 90.0, "driver_name": "SimulatedServo"}
```

Response:
```json
{"ok": true, "device_id": "servo_01", "value": 90.0, "driver": "SimulatedServo"}
```

### `POST /uri/transform` — Apply a data transform

```json
{"name": "normalize_temp", "data": [0.0, 23.5, 100.0]}
```

Response:
```json
{"name": "normalize_temp", "result": [0.0, 0.235, 1.0]}
```

### `POST /uri/status` — Query driver/device status

```json
{"device_id": "temp_01"}
```

Response:
```json
{"device_id": "temp_01", "driver": "SimulatedTemperature", "driver_status": "Online", "last_read": "...", "last_value": 23.5}
```

### `POST /uri/switch` — Switch active driver for a device

```json
{"device_id": "temp_01", "driver_name": "SimulatedTemperature"}
```

Response:
```json
{"ok": true, "device_id": "temp_01", "active_driver": "SimulatedTemperature"}
```

### `GET /uri/metrics` — Driver usage metrics

Response:
```json
{"metrics": [{"name": "SimulatedTemperature", "calls": 5, "errors": 0}, ...], "count": 4}
```

### `GET /uri/drivers` — List registered hardware drivers

Response:
```json
{"count": 4, "drivers": [{"name": "SimulatedTemperature", "driver_type": "builtin", "capabilities": ["sensor:temperature"], "status": "Online"}, ...]}
```

### `GET /uri/drivers/{name}` — Get driver metadata

Response:
```json
{"name": "SimulatedTemperature", "driver_type": "builtin", "capabilities": ["sensor:temperature"], "status": "Online", ...}
```

Error:
```json
{"error": "Driver 'Foo' not found"}
```

### `POST /uri/drivers/register` — Register a runtime hardware driver

```json
{
  "name": "MyGPIO", "driver_type": "Runtime",
  "capabilities": ["sensor:gpio", "actuator:gpio"],
  "config": {"pin": 17, "mode": "input"}
}
```

Response:
```json
{"ok": true, "name": "MyGPIO"}
```

### `POST /uri/drivers/unregister` — Unregister a runtime driver

```json
{"name": "MyGPIO"}
```

Response:
```json
{"ok": true}
```

Error:
```json
{"error": "Cannot unregister built-in driver 'SimulatedTemperature'"}
```

---

## Controller — Hardware Control Loops (feature `uri`)

Real-time control primitives: PID controllers, trajectory planning, safety monitors, emergency stop, coordinate transforms.

### `POST /controller/pid` — Create a PID controller

```json
{
  "id": "pid_arm_01", "kp": 1.5, "ki": 0.1, "kd": 0.05,
  "setpoint": 90.0, "dt": 0.01,
  "output_limits": [-100.0, 100.0]
}
```

Response:
```json
{"ok": true, "id": "pid_arm_01"}
```

### `GET /controller/pid` — List PID controllers

```json
{"controllers": [{"id": "pid_arm_01", "kp": 1.5, "ki": 0.1, "kd": 0.05, "setpoint": 90.0, "output_limits": [-100.0, 100.0]}], "count": 1}
```

### `POST /controller/pid/setpoint` — Update PID setpoint

```json
{"id": "pid_arm_01", "setpoint": 45.0}
```

Response:
```json
{"ok": true, "id": "pid_arm_01", "setpoint": 45.0}
```

### `POST /controller/pid/step` — Execute one PID control step

```json
{"id": "pid_arm_01", "measurement": 30.0}
```

Response:
```json
{"id": "pid_arm_01", "output": 65.25, "error": 15.0, "p_term": 22.5, "i_term": 1.5, "d_term": 41.25}
```

### `POST /controller/trajectory` — Create a trajectory plan

```json
{
  "id": "traj_pick_place", "interpolation": "Linear",
  "waypoints": [
    {"position": [0.0, 0.0, 0.0], "velocity": [0.0, 0.0, 0.0], "duration_secs": 1.0},
    {"position": [1.0, 0.0, 0.5], "velocity": [0.1, 0.0, 0.0], "duration_secs": 2.0},
    {"position": [1.0, 1.0, 0.5], "velocity": [0.0, 0.1, 0.0], "duration_secs": 2.0}
  ]
}
```

Response:
```json
{"ok": true, "id": "traj_pick_place"}
```

### `GET /controller/trajectory` — List trajectory plans

Response:
```json
{"trajectories": [{"id": "traj_pick_place", "current_index": 0, "status": "Idle", "waypoints": 3}], "count": 1}
```

### `POST /controller/trajectory/start` — Start trajectory execution

```json
{"id": "traj_pick_place"}
```

Response:
```json
{"ok": true, "id": "traj_pick_place", "status": "Running", "waypoint_0": {"position": [0.0, 0.0, 0.0]}}
```

### `POST /controller/trajectory/step` — Advance trajectory one waypoint

```json
{"id": "traj_pick_place"}
```

Response:
```json
{"ok": true, "id": "traj_pick_place", "next_waypoint": {"position": [1.0, 0.0, 0.5]}, "index": 1, "remaining": 1}
```

### `POST /controller/safety` — Create a safety monitor

```json
{
  "id": "safe_temp_01", "device_id": "temp_01",
  "condition": "ThresholdAbove", "threshold": 80.0
}
```

Supported conditions: `ThresholdAbove`, `ThresholdBelow`, `RateOfChange`, `Window`.

Response:
```json
{"ok": true, "id": "safe_temp_01"}
```

### `POST /controller/safety/check` — Run safety check

```json
{"device_id": "temp_01", "value": 85.0}
```

Response (triggered):
```json
{"safe": false, "triggered": [{"id": "safe_temp_01", "condition": "ThresholdAbove", "threshold": 80.0, "value": 85.0}]}
```

Response (passed):
```json
{"safe": true, "triggered": []}
```

### `POST /controller/safety/estop` — Trigger emergency stop

```json
{"device_id": "temp_01", "reason": "Temperature critical"}
```

Response:
```json
{"ok": true, "event": {"id": "...", "device_id": "temp_01", "event_type": "ESTOP_TRIGGERED", "message": "Temperature critical", "cleared": false}}
```

### `POST /controller/safety/estop/clear` — Clear emergency stop

```json
{"device_id": "temp_01"}
```

Response:
```json
{"ok": true, "device_id": "temp_01", "cleared": true}
```

### `GET /controller/safety/monitors` — List safety monitors

Response:
```json
{"monitors": [{"id": "safe_temp_01", "device_id": "temp_01", "condition": "ThresholdAbove", "threshold": 80.0, "triggered": false}], "count": 1}
```

### `GET /controller/safety/events/{device_id}` — Get safety events for a device

Response:
```json
{"events": [{"id": "...", "device_id": "temp_01", "event_type": "ESTOP_TRIGGERED", "message": "Temperature critical", "timestamp": "...", "cleared": false}], "count": 1}
```

### `POST /controller/transform` — Add a coordinate transform

```json
{
  "name": "base_to_world",
  "matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}
```

Response:
```json
{"ok": true, "name": "base_to_world"}
```

### `GET /controller/transform` — List coordinate transforms

Response:
```json
{"transforms": [{"name": "base_to_world"}], "count": 1}
```

### `GET /controller/stats` — Controller statistics

Response:
```json
{"stats": {"pid_count": 1, "trajectory_count": 1, "monitor_count": 1, "transform_count": 1, "total_events": 1, "active_estops": 1}}
```

---

## Grid — Decentralised Cognitive Grids

### `POST /grid/join` — Register a peer node

```json
{"node_id": "node-abc", "address": "192.168.1.10:7400"}
```

Response:
```json
{"ok": true, "node_id": "self-node-id"}
```

### `GET /grid/peers` — List connected peers

Response:
```json
{"peers": [{"node_id": "node-abc", "address": "192.168.1.10:7400", "status": "alive", ...}], "count": 1}
```

### `GET /grid/stats` — Grid statistics

Response:
```json
{"stats": {"peers": 3, "processes_migrated": 5, "heartbeats_received": 42, ...}}
```

### `POST /grid/heartbeat` — Send heartbeat

```json
{"node_id": "node-abc"}
```

Response:
```json
{"ok": true, "local_node_id": "self-node-id"}
```

### `POST /grid/migrate` — Migrate a process to another node

```json
{"pid": "proc-abc123", "target_node": "node-def"}
```

Response:
```json
{"ok": true, "frozen": {"pid": "proc-abc123", ...}}
```

### `POST /grid/receive` — Receive a migrating process

```json
{"pid": "proc-abc123", "manifest": {...}, "source_node": "node-def", "token_quota": 1000, "tokens_used": 50, "priority": 1, "active_l1_context": [...], "state_machine": {...}, "parent_pid": null, "goal_metric": 0.0, "created_at": "..."}
```

Response:
```json
{"ok": true, "pid": "proc-abc123"}
```

### `POST /grid/cipc/forward` — Forward a cIPC message to a remote node

```json
{"from": "proc-abc123", "to": "proc-def456", "payload": {"type": "request", ...}}
```

Response:
```json
{"ok": true, "message_id": "msg-..."}
```
