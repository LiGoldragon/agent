# INTENT — agent

*The daemon for the `agent` LLM-call component. Companion to `ARCHITECTURE.md`
and `Cargo.toml`. Maintenance: `primary/skills/repo-intent.md`.*

## Repo-scope only

This file carries only the intent that is FOR this `agent` daemon. Workspace
intent stays in `primary/INTENT.md`. The ordinary call contract is in
`signal-agent/INTENT.md`; the meta policy contract in
`meta-signal-agent/INTENT.md`.

## Why this repo exists

`agent` is the daemon of the agent triad: it receives a `Call(Prompt)` over its
ordinary socket and makes an HTTPS request to a configured provider's
OpenAI-compatible `/chat/completions` endpoint, returning a `Completion`. It is
the LLM-call substrate the gated Spirit guardian uses to judge intent.

## Stated psyche intent

The psyche authorized this build and made two shaping decisions, captured in
Spirit:

- *The `agent` component is built as an LLM-API-call component that makes
  provider HTTP API calls in an API style, not as an agent-harness front door;
  harness backends are deferred.* (Spirit `iucr`, Decision.)
- *The agent component models LLM providers as a generic OpenAI-compatible API
  with endpoint, model, and key as configuration, so adding a provider is
  configuration rather than a contract change.* (Spirit `f8k7`, Decision.)

Both decisions shape this daemon directly:

- **LLM API calls, not harness sessions.** The daemon's one external effect is a
  provider HTTP API call. It does not spawn or supervise Claude Code / Codex /
  Pi sessions; those harness backends are deferred and absent.
- **Providers are configuration.** A provider is a row in the engine's registry
  (name, endpoint, default model, API-key handle). The same
  `OpenAiCompatibleProvider` serves DeepSeek, MiMo, Kimi, GLM, and MiniMax —
  only the configured endpoint, model, and key differ. DeepSeek and MiMo are the
  first two configured providers.

## Principles

- **The HTTPS call is an async effect, never a blocking handler.** The provider
  call runs through the Nexus `CallProvider` effect plane, which the generated
  async runner awaits off the engine mailbox. No blocking IO inside an actor
  handler (`primary/skills/actor-systems.md`).
- **API keys are env-resolved handles, never hardcoded.** The configuration and
  registry carry only a key HANDLE (an environment-variable name); the secret
  value is resolved at call time and held only for the duration of one call. The
  agent never logs or persists the secret (`primary/skills/secrets.md`).
- **Binary-only daemon startup.** `agent-daemon` takes exactly one argument: a
  binary rkyv configuration file. It never parses NOTA, configuration included.
  Provider configuration arrives via the binary startup seed or authenticated
  meta-signal `ConfigureProvider`, never flags.
- **Fixture-without-network.** A `FixtureProvider` lets the daemon build and its
  round-trip test run with no live key and no network; the real reqwest call is
  behind the `live-provider` feature, and the live-network test gates on a key
  being present.

## Constraints (deferred / out of scope now)

- Harness backends (Claude Code / Codex / Pi sessions) are deferred — not built.
- Streaming (`StreamCall` / `CancelStream`) is contract-complete but the daemon
  replies `RequestUnimplemented` for now; the single-shot `Call` path is live.
- The provider registry is held in the engine; the durable redb (SEMA)
  projection of the registry is deferred (the SEMA plane is honestly stateless).

## See also

- `ARCHITECTURE.md` — the runtime triad, the provider call path, the registry.
- `../signal-agent/INTENT.md` — the ordinary call contract.
- `../meta-signal-agent/INTENT.md` — provider configuration + lifecycle.
- `primary/skills/component-triad.md`, `primary/skills/actor-systems.md`,
  `primary/skills/secrets.md`.
