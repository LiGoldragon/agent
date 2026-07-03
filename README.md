# agent

The LLM-API-call daemon: it receives a `Call(Prompt)` and makes an HTTPS request
to a configured provider's OpenAI-compatible `/chat/completions` endpoint,
returning a `Completion`. It is the LLM-call substrate the gated Spirit guardian
uses to judge intent — an LLM-API caller, not an agent harness (psyche scope).

A provider is a generic OpenAI-compatible API (endpoint + model + typed
secret-source reference); adding DeepSeek, MiMo, Kimi, GLM, MiniMax, or a local
subscription-backed server is configuration through `meta-signal-agent`, never
code. API keys are resolved by the daemon from configured backends such as
gopass, or `NoSecret` sends no Authorization header for a trusted loopback
server. Secrets are never hardcoded and never logged.

- `agent-daemon` — the long-lived process. One binary rkyv configuration
  argument; binds an ordinary (working) socket and a `0o600` meta socket.
- `agent` — the thin CLI: one NOTA `signal_agent::Input` argument, `AGENT_SOCKET`
  from the environment, NOTA reply on stdout.
- `agent-write-configuration` — the deploy/bootstrap text edge. The legacy
  `(AgentConfigurationWriteRequest ...)` shape leaves provider interaction
  logging disabled. To enable full JSONL provider interaction logging, use
  `(AgentConfigurationWriteRequestWithProviderInteractionLogging (<ordinary-socket> <meta-socket> <meta-mode> <database-path> <provider-seeds> (JsonLines <log-path>) <output-rkyv>))`.
  The log path must be separate from the agent database path.

Build offline with the fixture provider (default). The reqwest-backed real call
is behind `--features live-provider`; the live-network test gates on a key.

Read `ARCHITECTURE.md` for the runtime triad and the async provider-call effect,
and `INTENT.md` for the psyche-stated scope.
