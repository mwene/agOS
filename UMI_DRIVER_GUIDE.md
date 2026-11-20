# UMI Driver Guide

The Unified Model Interface (UMI) is the kernel's model boundary. Agent code
calls `kernel.umi.reason(...)`; it never talks to a provider SDK directly. That
keeps the scheduler, budgets, and governance layers model-agnostic and lets a
running kernel switch drivers without restarting processes.

Implementation: `agos-kernel/src/umi.rs` (interface and bookkeeping) and
`agos-kernel/src/llm.rs` (the live OpenAI-compatible driver).

## Driver slots

| Name | Type | Capability | Behaviour |
|------|------|-----------|-----------|
| `LocalEngine-v0` | built-in | 2 | Deterministic stub: prefixes each prompt line with `reasoned:` |
| `CloudGPT-HighReason` | built-in | 5 | Deterministic stub: prefixes with `deep_reasoned:` |
| `OpenAICompatible` | live | 8 | Real HTTP calls to an OpenAI-compatible endpoint |
| any registered name | runtime | caller-supplied | Deterministic stub with caller-supplied prefix |

The built-ins always exist. That is deliberate: the test suite and offline demos
must not depend on network access or API keys. The live driver only exists when
it is configured.

## Configuring the live driver

Set the environment before starting the kernel:

```bash
export AGOS_LLM_BASE_URL="https://api.openai.com/v1"   # or DeepSeek, Groq, vLLM, LM Studio...
export AGOS_LLM_API_KEY="sk-..."                       # omit for local runtimes
export AGOS_LLM_MODEL="gpt-4o-mini"
export AGOS_LLM_TIMEOUT_SECS="60"
export AGOS_LLM_MAX_TOKENS="1024"                      # optional
export AGOS_LLM_TEMPERATURE="0.2"                      # optional
```

`OPENAI_BASE_URL` and `OPENAI_API_KEY` work as fallbacks. The driver is created
when either a base URL or an API key is present; with neither, the kernel logs
that the live driver is disabled and stays on the stubs.

Local runtimes need no key:

```bash
# Ollama
export AGOS_LLM_BASE_URL="http://localhost:11434/v1"
export AGOS_LLM_MODEL="llama3.1"

# LM Studio / vLLM (also OpenAI-compatible)
export AGOS_LLM_BASE_URL="http://localhost:1234/v1"
export AGOS_LLM_MODEL="local-model"
```

The endpoint used is always `{AGOS_LLM_BASE_URL}/chat/completions`.

## Selecting a driver

```bash
# Hot-swap the active driver
curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "OpenAICompatible"}'

# Inspect drivers, config (key redacted), and counters
curl http://127.0.0.1:7400/umi/drivers
curl http://127.0.0.1:7400/umi/driver/OpenAICompatible
curl http://127.0.0.1:7400/umi/metrics
```

Switching only changes the atomic active slot; running processes keep their
identity and their next `reason()` call uses the new driver.

## Runtime configuration

`OpenAICompatible` can be reconfigured without a restart:

```bash
curl -X POST http://127.0.0.1:7400/umi/driver/config/OpenAICompatible \
  -H 'Content-Type: application/json' \
  -d '{"config": {"model": "deepseek-chat", "timeout_secs": "30"}}'
```

Accepted keys: `base_url`, `model`, `api_key` (empty string clears it),
`timeout_secs`, `max_tokens`, `temperature`. Unknown keys are rejected and the
old config is kept. An API key is never returned by the API; it is reported as
`"***"`.

## Prompt mapping

The kernel builds `prompt_vector` as an instruction-first list:

- one element → a single `user` message;
- two or more → the first element becomes the `system` message and the rest are
  joined with blank lines into the `user` message.

This matches how call sites in `orchestrator.rs`, `marketplace.rs`, and
`embodiment.rs` construct prompts (instructions first, then task data, then an
output-format hint).

## Response handling

The completion content is split on newlines and trimmed, because downstream
consumers parse line-oriented output (for example `RESOLVED: ...` in the
embodiment layer and `STEP_ID | ... | ...` in the marketplace).

Token counts come from the provider's `usage` block when present, otherwise
from a character-count estimate. Per-driver counters are updated on every call
and exposed via `/umi/metrics`.

On provider failure the kernel does not fail the HTTP request. `/umi/reason`
returns:

```json
{
  "output": [],
  "tokens_used": 0,
  "model": "OpenAICompatible",
  "mode": "UMI_ERROR",
  "error": "model API returned HTTP 401: ..."
}
```

Callers treat an empty output as "no answer", which keeps workflows and goal
pipelines from stalling on a transient provider error.

## Registering a stub driver at runtime

```bash
curl -X POST http://127.0.0.1:7400/umi/driver/register \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "MyStub",
    "capability_level": 3,
    "token_cost": 4,
    "prefix": "mystub",
    "embed_modulus": 50000,
    "max_output": 8,
    "config": {"note": "deterministic test driver"}
  }'

curl -X POST http://127.0.0.1:7400/umi/switch \
  -H 'Content-Type: application/json' \
  -d '{"driver_name": "MyStub"}'
```

Registered drivers are in-memory only and disappear on restart. `LocalEngine-v0`,
`CloudGPT-HighReason`, and `OpenAICompatible` are reserved names.

## Adding a different protocol

`llm.rs` is intentionally small so a second protocol can be added beside it:

1. Create a module that exposes the same shape as `LlmDriver`: a constructor
   from config, `reason(&self, &[String]) -> Result<LlmCompletion, String>`, and
   counters.
2. Hold it as another optional field on `UnifiedModelInterface`, with its own
   active sentinel and a `DriverInfo` entry marked `driver_type: "live"`.
3. Keep the deterministic built-ins as the default path so `cargo test` never
   needs the network.
4. Add a test that points the driver at a local axum server, the same way the
   tests in `llm.rs` do.

## Known limits

- No streaming; `stream: false` is always sent.
- Embeddings still use the deterministic local hash (`/umi/embed`); the live
  driver covers reasoning only.
- The `schema` argument to `reason()` is not yet mapped to a provider
  `response_format`/JSON mode.
- Only one runtime stub driver can be active at a time; the kernel selects the
  first registered entry.
