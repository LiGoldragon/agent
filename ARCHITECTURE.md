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

## Runtime triad

The daemon is schema-derived on the emitted runtime. The working tier's
`Input`/`Output` come from the dependency contract `signal-agent`; the
daemon-local plane schemas declare the Nexus and SEMA planes.

| Plane | Schema | Role |
|---|---|---|
| Signal | `signal-agent` (dependency `WireContract`) | the wire surface; emitted spine decodes `Input`, encodes `Output` |
| Nexus | `schema/nexus.schema` | the decision + the one external effect: `CallProvider` |
| SEMA | `schema/sema.schema` | honestly stateless (registry held in the engine; redb projection deferred) |

`AgentEngine` is the data-bearing noun the generated `NexusEngine` and
`SemaEngine` traits attach to. It owns the provider registry, the boxed
`Provider`, and the boxed `KeySource`.

## The call path — the async effect seam

```text
Call(Prompt)  --Signal-->  NexusWork::SignalArrived
  -> NexusEngine::decide  ->  CommandEffect(CallProvider(Prompt))
  -> generated async runner awaits NexusEngine::run_effect
       -> registry.resolve(prompt) -> ProviderCall (endpoint, model, key, messages)
       -> Provider::complete(call).await        <-- the only network IO
       -> ProviderOutcome::{Completed | Rejected}
  -> NexusWork::EffectCompleted
  -> NexusEngine::decide  ->  ReplyToSignal(Output::Completed | CallRejected)
Output  --Signal-->  wire
```

The HTTPS call is the Nexus `CallProvider` effect. The generated runner awaits
`run_effect` off the engine mailbox — no blocking inside an actor handler
(`primary/skills/actor-systems.md` §"Blocking is a design bug"). This is the
load-bearing discipline: the provider call is an async effect, never an inline
blocking await.

## The provider plane

`Provider` is a dyn-compatible async trait (`complete(ProviderCall) ->
ProviderCompletionFuture`). Two implementations:

- `FixtureProvider` — deterministic, no network, no key. Lets the daemon build
  and the round-trip test run offline. The default build uses it.
- `OpenAiCompatibleProvider` (feature `live-provider`) — the reqwest-backed
  call. One client serves every configured provider; only endpoint, model, and
  authorization differ. It posts the OpenAI chat-completions body (system +
  transcript, `temperature`, `max_tokens`, `response_format: json_object` when
  the prompt's `OutputMode` is `JsonObject`) with a bearer token only when the
  resolved authorization carries one. `NoSecret` sends no Authorization header
  for trusted loopback OpenAI-compatible servers.

## The provider registry — policy state

`ProviderRegistry` holds `ProviderEntry` rows (name, endpoint, default model,
secret source) plus a default. `resolve(prompt)` picks the provider (prompt's
named provider, else the default), the model (prompt's, else the provider
default), and resolves the secret source through a `KeySource`. The production
`KeySource` is `SystemKeySource`, which supports `Environment`, `Gopass`, and
`File` backends. The `NoSecret` source bypasses key resolution and is intended
for a local OpenAI-compatible server such as `http://127.0.0.1:18080/v1` with
model `gpt-5.5`; if that local server is started with its own API-key gate, use
`Environment` or `File` instead. Tests inject a literal key source so a fixture
call needs no process environment.

The registry is configured through the meta tier (`handle_meta_connection`
decodes `meta_signal_agent::Input`, mutates the registry) and seeded at startup
from the binary configuration's `bootstrap_providers`.

## Provider interaction log

Full provider interaction logging is an explicit startup configuration setting,
disabled by default. The binary `AgentDaemonConfiguration` carries
`ProviderInteractionLogging::{Disabled | JsonLines(path)}`; the text bootstrap
edge exposes this as
`(AgentConfigurationWriteRequestWithProviderInteractionLogging (... (JsonLines <log-path>) ...))`.
The old `(AgentConfigurationWriteRequest ...)` writer shape still produces
`Disabled`.

When enabled, `AgentEngine` appends one JSON object per provider attempt to the
configured JSONL file. The log is outside the agent database: startup rejects a
logging path equal to the configured database path. Each record includes the
provider name, endpoint, model, redacted authorization metadata, the exact
OpenAI-compatible request body, response status/body when available,
provider-level success/failure, NOTA validation outcome, and the daemon outcome.
Bearer values and generated Authorization headers are redacted; `NoSecret`
records no Authorization header.

## Two authority tiers

- **Working tier** (`signal-agent`): `Call` / `StreamCall` / `CancelStream`. The
  generated spine runs `handle_working_input` -> `AgentEngine::handle`.
- **Meta tier** (`meta-signal-agent`): `ConfigureProvider` / `RetireProvider` /
  `SetDefaultProvider` / `Start` / `Stop`, on a `0o600` socket. Decoded by the
  component-owned `handle_meta_connection`; mutates the registry.

## The one-argument rule

`agent-daemon` takes exactly one argument: a binary rkyv `AgentDaemonConfiguration`
(ordinary socket, meta socket + mode, database path, optional provider seeds). It
rejects inline NOTA and `.nota` paths and never parses NOTA. The `agent` CLI is
the thin text-to-Signal client: one NOTA `Input` argument, `AGENT_SOCKET` from
the environment, binary frame to the daemon, NOTA reply on stdout.

## Deferred

- Harness backends (Claude Code / Codex / Pi) — out of scope by psyche decision.
- `StreamCall` / `CancelStream` — contract-complete; the daemon replies
  `RequestUnimplemented` until the streaming runner lands.
- The redb (SEMA) durable projection of the provider registry — the registry is
  in-memory, re-supplied by meta `Configure` on restart.
- The contract dependencies are consumed from `signal-agent` and
  `meta-signal-agent` main. Contract source and generated wire nouns stay in
  those repos; this daemon imports them and emits only its runtime planes.

## Code map

```text
schema/nexus.schema        Nexus plane (decision + CallProvider effect)
schema/sema.schema         SEMA plane (stateless)
build.rs                   two-tier daemon shape (signal-agent working + meta tier)
src/schema/{nexus,sema,daemon}.rs   generated runtime (freshness-checked)
src/engine.rs              AgentEngine: NexusEngine + SemaEngine impls
src/provider.rs            Provider trait, FixtureProvider, OpenAiCompatibleProvider
src/registry.rs            ProviderRegistry, KeySource, ProviderEntry
src/interaction_log.rs     Disabled-by-default JSONL provider interaction log
src/config.rs              binary rkyv AgentDaemonConfiguration
src/schema_daemon.rs       ComponentDaemon impl + meta-tier projection
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
