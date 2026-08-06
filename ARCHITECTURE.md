# agent — Architecture

`agent` is the daemon of the agent triad (`agent` runtime, `signal-agent`
ordinary contract, `meta-signal-agent` meta policy contract). It is an
LLM-API-call component: it receives a `Call(Prompt)` and makes an HTTPS request
to a configured provider's OpenAI-compatible `/chat/completions` endpoint.

It cites `primary/skills/component-triad.md` and states only the
component-specific shape below; the universal invariants live in that skill.

## Direction

Per psyche Spirit `iucr` / `f8k7`: an LLM-API caller, not an agent harness.
Harness backends are deferred. Providers are configuration: a generic
OpenAI-compatible API (endpoint + model + typed secret-source reference), so
adding one is a `ConfigureProvider` message, never code.

## Runtime boundary

The daemon consumes the authority-sealed `signal-agent` and
`meta-signal-agent` contracts directly. It owns no schema, generated interface,
or parallel naming boundary. `AgentEngine` owns the provider registry, the
boxed `Provider`, and the boxed `KeySource`; `AgentRuntime` binds each socket
authority to its canonical contract.

## The call path — the async effect seam

```text
Call(Prompt)  --Signal-->  AgentEngine::handle
  -> registry.resolve(prompt) -> ProviderCall (endpoint, model, key, messages)
  -> Provider::complete(call).await              <-- the only network IO
  -> Output::{Completed | CallRejected}
Output  --Signal-->  wire
```

The HTTPS call is an asynchronous effect; no blocking network operation enters
the runtime.

## The provider plane

`Provider` is a dyn-compatible async trait (`complete(ProviderCall) ->
ProviderCompletionFuture`). Two implementations:

- `FixtureProvider` — deterministic, no network, no key. Lets the daemon build
  and the round-trip test run offline. The default build uses it.
- `OpenAiCompatibleProvider` (feature `live-provider`) — the reqwest-backed
  call. One client serves every configured provider; only endpoint, model, and
  authorization differ. It posts the OpenAI chat-completions body (system +
  transcript, model-compatible optional parameters such as `temperature` and
  `max_tokens`) with a bearer token only when the resolved authorization carries
  one. `NoSecret` sends no Authorization header for trusted loopback
  OpenAI-compatible servers.

## The provider registry — policy state

`ProviderRegistry` holds `ProviderEntry` rows (name, endpoint, default model,
secret source) plus a default. `resolve(prompt)` picks the provider (prompt's
named provider, else the default), the model (prompt's, else the provider
default), and resolves the secret source through a `KeySource`. The production
`KeySource` is `SystemKeySource`, which supports `Environment`, `Gopass`, and
`File` backends. The `NoSecret` source bypasses key resolution and is intended
for a local OpenAI-compatible server such as `http://127.0.0.1:18080/v1` with
model `gpt-5.4-mini` (the current local judge/eval default). The same endpoint
may also advertise `gpt-5.5` for explicit fallback runs. If that local server is
started with its own API-key gate, use `Environment` or `File` instead. Tests
inject a literal key source so a fixture call needs no process environment.

The registry is configured through the meta tier (which decodes the canonical
meta request, then mutates the registry) and seeded at startup
from the binary configuration's `bootstrap_providers`.

## Two authority tiers

- **Working tier** (`signal-agent`): `Call` / `StreamCall` / `CancelStream`.
  `AgentRuntime` decodes the bound frame and invokes `AgentEngine::handle`.
- **Meta tier** (`meta-signal-agent`): `ConfigureProvider` / `RetireProvider` /
  `SetDefaultProvider` / `Start` / `Stop`, on a `0o600` socket. Decoded by the
  the concrete meta turn; mutates the registry.

## The one-argument rule

`agent-daemon` takes exactly one argument: a binary rkyv `AgentDaemonConfiguration`
(ordinary socket, meta socket + mode, database path, optional provider seeds). It
rejects inline DOTOS and `.dotos` paths and never parses DOTOS. The `agent` CLI is
the thin text-to-Signal client: one DOTOS `Input` argument, `AGENT_SOCKET` from
the environment, binary frame to the daemon, DOTOS reply on stdout.

## Deferred

- Harness backends (Claude Code / Codex / Pi) — out of scope by psyche decision.
- `StreamCall` / `CancelStream` — contract-complete; the daemon replies
  `RequestUnimplemented` until the streaming runner lands.
- The durable projection of the provider registry — the registry is
  in-memory, re-supplied by meta `Configure` on restart.
- The contract dependencies are consumed from `signal-agent` and
  `meta-signal-agent` at exact revisions. Contract authority and its Rust
  projection stay in those repos; this daemon owns neither.

## Code map

```text
src/daemon.rs              concrete two-authority socket runtime
src/engine.rs              AgentEngine: contract input -> provider effect -> contract output
src/provider.rs            Provider trait, FixtureProvider, OpenAiCompatibleProvider
src/registry.rs            ProviderRegistry, KeySource, ProviderEntry
src/config.rs              binary rkyv AgentDaemonConfiguration
src/client.rs              CLI daemon client
src/bin/agent.rs           CLI binary
src/bin/agent_daemon.rs    daemon binary
tests/fixture_round_trip.rs  offline fixture round-trip witness
```

## See also

- `primary/skills/component-triad.md` — the universal triad invariants.
- `primary/skills/actor-systems.md` — no blocking in handlers; the effect seam.
- `primary/skills/secrets.md` — secret-source references, never secret values.
- `../signal-agent/ARCHITECTURE.md`, `../meta-signal-agent/ARCHITECTURE.md`.
