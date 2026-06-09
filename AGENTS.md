# agent agent notes

Read `/home/li/primary/AGENTS.md` first, then this repo's `INTENT.md` and
`ARCHITECTURE.md`.

`agent` is the daemon of the agent triad — an LLM-API-call component that makes
OpenAI-compatible provider HTTP API calls. It is NOT an agent harness (psyche
Spirit `iucr`, `f8k7`); harness backends are deferred.

Before editing daemon Rust, read `/home/li/primary/skills/component-triad.md`,
`/home/li/primary/skills/actor-systems.md`, `/home/li/primary/skills/kameo.md`,
and `/home/li/primary/skills/secrets.md`.

Load-bearing rules for this repo:

- The provider HTTPS call is the Nexus `CallProvider` async effect, awaited off
  the engine mailbox. Never a blocking await inside an actor handler.
- API keys are env-resolved handles, never hardcoded; the secret value is never
  logged or persisted.
- The daemon takes exactly one binary rkyv argument and never parses NOTA.
- Edit `schema/nexus.schema` / `schema/sema.schema` and regenerate
  (`AGENT_UPDATE_SCHEMA_ARTIFACTS=1 cargo build`); never hand-edit
  `src/schema/*.rs`.
- `cargo test` runs the offline fixture round-trip; the live-network test gates
  on `--features live-provider` plus a key in the environment.
