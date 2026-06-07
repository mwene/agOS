# UMI Driver Developer Guide

This guide covers how to develop, register, and hot-swap AI model drivers for the **Unified Model Interface (UMI)** — the abstraction layer that decouples agOS agents from any specific inference engine.

---

## Table of Contents

- [What is UMI?](#what-is-umi)
- [Driver Architecture](#driver-architecture)
- [The DriverState Struct](#the-driverstate-struct)
- [The UnifiedModelInterface Struct](#the-unifiedmodelinterface-struct)
- [Implementing a Driver](#implementing-a-driver)
- [Hot-Swap Mechanism](#hot-swap-mechanism)
- [UMI Syscalls Reference](#umi-syscalls-reference)
- [End-to-End Walkthrough: Adding an Ollama Driver](#end-to-end-walkthrough-adding-an-ollama-driver)
- [End-to-End Walkthrough: Adding a Mock/Test Driver](#end-to-end-walkthrough-adding-a-mocktest-driver)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

---

## What is UMI?

The **Unified Model Interface** is the driver abstraction layer in agOS. Just as UNIX abstracts hardware devices through device files, UMI abstracts AI model inference through four syscalls:

| Syscall | Function | Description |
|---------|----------|-------------|
| `SYS_REASON` | `reason()` | Forward a prompt vector to the model, return generated output |
| `SYS_EMBED` | `embed()` | Convert text data into embedding vectors |
| `SYS_COMPRESS` | `compress()` | Summarize or compress context data |
| `SYS_SWITCH` | `hot_swap()` | Atomically replace the active driver |

Agents call these syscalls via the REST API. The UMI routes them to whichever driver is currently active — swapping drivers requires zero agent-facing changes.

```
  Agent A                 Agent B
    │                        │
    └──────┬─────────────────┘
           │
    ┌──────▼──────┐
    │    UMI      │  ← routes to active driver
    └──────┬──────┘
           │
    ┌──────┼──────┬──────────────┐
    │      │      │              │
    ▼      ▼      ▼              ▼
 Local   Cloud  Ollama   Custom Driver...
Engine-v0 GPT    Driver   (yours here)
 (cap 2) (cap 5)
```

---

## Driver Architecture

Each driver is a **compile-time static singleton** implementing a `DriverState` struct. Drivers are selected at runtime via an atomic index switch — no trait objects, no `dyn`, no heap allocation for dispatch.

```
UnifiedModelInterface
├── active: AtomicU32  ← 0 = DRIVER_LOCAL, 1 = DRIVER_CLOUD, ...
├── reason() ─────────────→ self.driver().reason(...)
├── embed()  ─────────────→ self.driver().embed(...)
├── compress() ──────────→ self.driver().compress(...)
└── hot_swap(name) ─────→ self.active.store(new_idx, Relaxed)
```

### File Layout

Drivers live in `agos-kernel/src/umi.rs`. All driver logic is in a single file — adding a new driver means:

1. Define a new `static DriverState`.
2. Assign it a unique `u32` constant.
3. Add a match arm in `driver()` and `hot_swap()`.
4. Implement the syscall methods.

---

## The DriverState Struct

```rust
struct DriverState {
    name: &'static str,          // Human-readable driver name (e.g. "LocalEngine-v0")
    capability_level: u32,       // Model capability rating (higher = more capable)
    calls: AtomicU64,            // Total inference calls (tracked for metrics)
    total_tokens: AtomicU64,     // Total tokens consumed (tracked for metrics)
    token_cost: u64,             // Cost multiplier per prompt element
    prefix: &'static str,        // Output prefix (e.g. "reasoned" or "deep_reasoned")
    embed_modulus: u64,          // Hash modulus for deterministic embedding
    max_output: usize,           // Max output lines per reason call
}
```

| Field | Type | Purpose |
|-------|------|---------|
| `name` | `&'static str` | Driver identifier. Must match `hot_swap()` string. |
| `capability_level` | `u32` | Relative capability. Used by agents to decide which driver to use. |
| `calls` | `AtomicU64` | Auto-incremented on each syscall. Exposed via `get_metrics()`. |
| `total_tokens` | `AtomicU64` | Sum of tokens consumed across all calls. Exposed via `get_metrics()`. |
| `token_cost` | `u64` | Cost per prompt element. Higher values = more token budget consumed per call. |
| `prefix` | `&'static str` | Prepended to each output line to distinguish driver provenance. |
| `embed_modulus` | `u64` | Controls embedding value range. Higher = finer granularity. |
| `max_output` | `usize` | Caps the number of output lines. Prevents runaway generation. |

All fields except the atomics must be `const`-compatible because `static` requires compile-time initialization.

---

## The UnifiedModelInterface Struct

```rust
pub struct UnifiedModelInterface {
    active: AtomicU32,
}
```

The entire UMI state is a single atomic integer — the index of the active driver. This makes hot-swap an **O(1) atomic store** with no locks.

### Driver Resolution

```rust
fn driver(&self) -> &'static DriverState {
    match self.active.load(Ordering::Relaxed) {
        DRIVER_CLOUD => &CLOUD_DRIVER,
        _ => &LOCAL_DRIVER,
    }
}
```

Every syscall resolves the active driver through this method, then delegates to the driver's implementation.

---

## Implementing a Driver

Follow these steps to add a new driver.

### Step 1: Define a Unique Constant

Add a new `u32` constant at the top of `umi.rs`:

```rust
const DRIVER_LOCAL: u32 = 0;
const DRIVER_CLOUD: u32 = 1;
const DRIVER_OLLAMA: u32 = 2;  // new
```

### Step 2: Create the Static DriverState

```rust
static OLLAMA_DRIVER: DriverState = DriverState {
    name: "Ollama-Llama3",
    capability_level: 4,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: 3,
    prefix: "llama",
    embed_modulus: 50000,
    max_output: 15,
};
```

### Step 3: Add a Match Arm in `driver()`

```rust
fn driver(&self) -> &'static DriverState {
    match self.active.load(Ordering::Relaxed) {
        DRIVER_CLOUD => &CLOUD_DRIVER,
        DRIVER_OLLAMA => &OLLAMA_DRIVER,  // new
        _ => &LOCAL_DRIVER,
    }
}
```

### Step 4: Add a Match Arm in `hot_swap()`

```rust
pub fn hot_swap(&self, driver_name: &str) -> Result<(), String> {
    match driver_name {
        "LocalEngine-v0" => { self.active.store(DRIVER_LOCAL, Ordering::Relaxed); Ok(()) }
        "CloudGPT-HighReason" => { self.active.store(DRIVER_CLOUD, Ordering::Relaxed); Ok(()) }
        "Ollama-Llama3" => { self.active.store(DRIVER_OLLAMA, Ordering::Relaxed); Ok(()) }  // new
        _ => Err(format!("Driver '{}' not found", driver_name)),
    }
}
```

### Step 5: Implement Syscall Logic

The default implementations in `umi.rs` are **deterministic stubs** — they use hash-based embeddings and prefix-based reasoning output. For a real driver, replace the logic in `reason()`, `embed()`, and `compress()`.

Each method follows the same pattern:

```rust
pub async fn reason(&self, prompt_vector: &[String], schema: &Value) -> UMIResult {
    let d = self.driver();              // resolve active driver
    d.calls.fetch_add(1, Relaxed);       // increment metrics
    let tokens = /* compute tokens */;
    d.total_tokens.fetch_add(tokens, Relaxed);
    // ... driver-specific inference ...
    UMIResult { output, tokens_used, model, mode }
}
```

### Step 6: Update Metrics

The `get_metrics()` method returns `Vec<DriverMetrics>` for all drivers. Add your driver to the list:

```rust
pub fn get_metrics(&self) -> Vec<DriverMetrics> {
    vec![
        metrics_for(&LOCAL_DRIVER),
        metrics_for(&CLOUD_DRIVER),
        metrics_for(&OLLAMA_DRIVER),  // new
    ]
}
```

### Step 7: Register in the Kernel Constructor

In `kernel.rs`, the UMI is created with default drivers. If your driver needs initialization (e.g., connecting to a service), add it to `agOSKernel::new()`:

```rust
pub fn new() -> Self {
    let umi = UnifiedModelInterface::new();
    // Initialize your driver here if needed.
    agOSKernel { umi, ... }
}
```

---

## Hot-Swap Mechanism

Hot-swap is the key architectural capability of UMI — changing the active model driver **at runtime with zero agent disruption**.

### How It Works

```
Before:              After:
┌────────────┐       ┌────────────┐
│  active: 0 │──────>│  active: 2 │
│  (Local)   │ swap  │  (Ollama)  │
└────────────┘       └────────────┘
     │                     │
     ▼                     ▼
 LocalEngine-v0       Ollama-Llama3
```

The swap is a single `AtomicU32::store(Ordering::Relaxed)` — O(1), lock-free, wait-free.

### REST API

```bash
# Switch to Ollama driver
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "Ollama-Llama3"}'

# Response
{"ok": true, "active_driver": "Ollama-Llama3", "capability": 4}
```

### Safety Considerations

- **Ordering::Relaxed** is safe here because there is no cross-thread dependency on the driver index — each request reads the index once at the start of the syscall.
- Old drivers continue to serve any in-flight requests that already resolved `driver()` before the store.
- No driver state is migrated on swap. The new driver starts fresh.

---

## UMI Syscalls Reference

### SYS_REASON — `reason(prompt_vector, schema) -> UMIResult`

| Parameter | Type | Description |
|-----------|------|-------------|
| `prompt_vector` | `&[String]` | Array of prompt elements |
| `schema` | `&Value` | JSON schema for structured output (optional) |

| Return Field | Type | Description |
|-------------|------|-------------|
| `output` | `Vec<String>` | Generated output lines |
| `tokens_used` | `u64` | Tokens consumed by this call |
| `model` | `String` | Driver name |
| `mode` | `String` | Always `"SYS_REASON"` |

**Default implementation**: Prepends `prefix` to each prompt element, caps at `max_output` lines. Token cost = `prompt_vector.len() * token_cost`.

**Real implementation**: Send `prompt_vector` to the model API, collect generated text, count actual tokens.

**REST**:

```bash
curl -X POST http://127.0.0.1:7400/umi/reason \
  -H 'Content-Type: application/json' \
  -d '{"prompt_vector": ["analyze", "financial_data"]}'
```

### SYS_EMBED — `embed(data) -> Vec<f64>`

| Parameter | Type | Description |
|-----------|------|-------------|
| `data` | `&[String]` | Text strings to embed |

**Default implementation**: Hashes each string and normalizes to `[0, 1)` using `embed_modulus`.

**Real implementation**: Call the model's embedding API endpoint.

**REST**:

```bash
curl -X POST http://127.0.0.1:7400/umi/embed \
  -H 'Content-Type: application/json' \
  -d '{"data": ["ledger_2024", "balance_verification"]}'
```

### SYS_COMPRESS — `compress(data) -> Value`

| Parameter | Type | Description |
|-----------|------|-------------|
| `data` | `&[String]` | Data to compress/summarize |

**Default implementation**: Returns metadata about the data (length, driver, timestamp).

**Real implementation**: Summarize the data using the model.

**REST**:

```bash
curl -X POST http://127.0.0.1:7400/umi/compress \
  -H 'Content-Type: application/json' \
  -d '{"data": ["long context to compress..."]}'
```

### SYS_SWITCH — `hot_swap(driver_name) -> Result<(), String>`

| Parameter | Type | Description |
|-----------|------|-------------|
| `driver_name` | `&str` | Must match a static driver's `name` field |

**REST**:

```bash
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "CloudGPT-HighReason"}'
```

---

## End-to-End Walkthrough: Adding an Ollama Driver

This walkthrough adds a real driver that calls a local [Ollama](https://ollama.ai) instance for inference.

### Step 1: Add the HTTP Client Dependency

In `agos-kernel/Cargo.toml`:

```toml
reqwest = { version = "0.12", features = ["json"] }
```

### Step 2: Define the Driver in umi.rs

```rust
const DRIVER_OLLAMA: u32 = 2;

static OLLAMA_DRIVER: DriverState = DriverState {
    name: "Ollama-Llama3",
    capability_level: 4,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: 3,
    prefix: "llama",
    embed_modulus: 50000,
    max_output: 15,
};
```

### Step 3: Add HTTP Client to UMI

Add an `ollama_client` field and initialization:

```rust
pub struct UnifiedModelInterface {
    active: AtomicU32,
    ollama_client: Option<reqwest::Client>,
}

impl UnifiedModelInterface {
    pub fn new() -> Self {
        UnifiedModelInterface {
            active: AtomicU32::new(DRIVER_LOCAL),
            ollama_client: Some(reqwest::Client::new()),
        }
    }
}
```

### Step 4: Implement Real reason()

```rust
pub async fn reason(&self, prompt_vector: &[String], schema: &Value) -> UMIResult {
    let d = self.driver();
    d.calls.fetch_add(1, Ordering::Relaxed);

    if d.name != "Ollama-Llama3" {
        // fallback to deterministic stub for non-Ollama drivers
        let tokens = (prompt_vector.len() as u64).max(1) * d.token_cost;
        d.total_tokens.fetch_add(tokens, Ordering::Relaxed);
        let output: Vec<String> = prompt_vector.iter()
            .take(d.max_output)
            .map(|s| format!("{}:{}", d.prefix, s))
            .collect();
        return UMIResult { output, tokens_used: tokens, model: d.name.to_string(), mode: "SYS_REASON".to_string() };
    }

    // Ollama inference
    if let Some(client) = &self.ollama_client {
        let prompt = prompt_vector.join("\n");
        let body = serde_json::json!({
            "model": "llama3.2",
            "prompt": prompt,
            "stream": false,
        });
        let resp = client
            .post("http://localhost:11434/api/generate")
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) => {
                let json: serde_json::Value = r.json().await.unwrap_or_default();
                let text = json.get("response").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let tokens = (text.len() / 4) as u64;
                d.total_tokens.fetch_add(tokens, Ordering::Relaxed);
                UMIResult {
                    output: vec![text],
                    tokens_used: tokens,
                    model: d.name.to_string(),
                    mode: "SYS_REASON".to_string(),
                }
            }
            Err(e) => UMIResult {
                output: vec![format!("Ollama error: {}", e)],
                tokens_used: 0,
                model: d.name.to_string(),
                mode: "SYS_REASON".to_string(),
            },
        }
    } else {
        UMIResult { output: vec!["No Ollama client".into()], tokens_used: 0, model: d.name.to_string(), mode: "SYS_REASON".to_string() }
    }
}
```

### Step 5: Add Match Arms in driver() and hot_swap()

```rust
fn driver(&self) -> &'static DriverState {
    match self.active.load(Ordering::Relaxed) {
        DRIVER_CLOUD => &CLOUD_DRIVER,
        DRIVER_OLLAMA => &OLLAMA_DRIVER,
        _ => &LOCAL_DRIVER,
    }
}

pub fn hot_swap(&self, driver_name: &str) -> Result<(), String> {
    match driver_name {
        "Ollama-Llama3" => { self.active.store(DRIVER_OLLAMA, Ordering::Relaxed); Ok(()) }
        // ... existing arms ...
        _ => Err(format!("Driver '{}' not found", driver_name)),
    }
}
```

### Step 6: Test

```bash
# Start kernel with Ollama support
cd agos-kernel && cargo run &

# Switch to Ollama driver
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "Ollama-Llama3"}'

# Reason with the remote model
curl -X POST http://127.0.0.1:7400/umi/reason \
  -H 'Content-Type: application/json' \
  -d '{"prompt_vector": ["What is the capital of France?"]}'

# Switch back to local
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "LocalEngine-v0"}'
```

---

## End-to-End Walkthrough: Adding a Mock/Test Driver

For testing, you might want a deterministic mock driver that returns predictable outputs.

```rust
const DRIVER_MOCK: u32 = 3;

static MOCK_DRIVER: DriverState = DriverState {
    name: "Mock-TestDriver",
    capability_level: 1,
    calls: AtomicU64::new(0),
    total_tokens: AtomicU64::new(0),
    token_cost: 1,
    prefix: "mock",
    embed_modulus: 100,
    max_output: 3,
};
```

In `reason()`, check for mock driver:

```rust
if d.name == "Mock-TestDriver" {
    return UMIResult {
        output: vec!["mock: predictable output".to_string()],
        tokens_used: 1,
        model: d.name.to_string(),
        mode: "SYS_REASON".to_string(),
    };
}
```

This is useful for integration tests where you need deterministic behavior.

---

## Best Practices

1. **Keep driver() Fast** — `driver()` is called on every syscall. The match pattern is O(1) — avoid allocation or I/O in this path.
2. **Atomic Metrics** — Always use `Ordering::Relaxed` for counters. Exact consistency isn't needed for metrics.
3. **Token Accounting** — Report actual tokens from the model API when possible. The token budget is enforced by the scheduler.
4. **Error Handling** — Never panic in a driver method. Return an `UMIResult` with an error message in `output`.
5. **Capability Levels** — Use consistent capability ratings:
   - 1 = deterministic/mock
   - 2 = small local model (e.g., Phi-3, TinyLlama)
   - 3 = medium local (e.g., Mistral 7B)
   - 4 = large local (e.g., Llama 3 70B)
   - 5 = cloud API (e.g., GPT-4, Claude 3)
   - 6+ = future/hyperscale
6. **Async Only for Real I/O** — The default implementations are synchronous. Only make methods `async` if the driver does actual network I/O.
7. **Hot-Swap Gracefully** — Design drivers so they can be swapped in/out at any time. No driver should hold exclusive access to shared state.

---

## Troubleshooting

### Driver not found

The `hot_swap()` name must exactly match the static driver's `name` field. Check for typos and case sensitivity.

### No metrics appearing

Ensure the driver's `calls` and `total_tokens` atomics are being incremented:

```bash
curl http://127.0.0.1:7400/umi/metrics
```

### Performance concerns with atomic dispatch

The atomic load + match is ~1ns — negligible compared to the ~100ms+ cost of a single model inference call.

### Same driver behavior after swap

Verify the swap succeeded:

```bash
curl http://127.0.0.1:7400/umi/metrics
# Check "active_driver" field
```

### Compile errors with static initialization

`static` requires all fields to be `const`-compatible. `AtomicU64::new(val)` is const, so this works. Avoid non-const initializers like `String::new()` or `Vec::new()` in `DriverState`.

---

## Reference: Complete Driver Checklist

- [ ] Unique `u32` constant defined
- [ ] `static DriverState` with all fields initialized
- [ ] Match arm in `driver()` method
- [ ] Match arm in `hot_swap()` method
- [ ] Syscall logic handles the new driver correctly
- [ ] `get_metrics()` includes the new driver
- [ ] Kernel compiles with `cargo build`
- [ ] Hot-swap works via REST API
- [ ] All existing drivers still work after swap
