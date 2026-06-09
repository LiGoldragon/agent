# agent

The LLM-API-call daemon: it receives a `Call(Prompt)` and makes an HTTPS request
to a configured provider's OpenAI-compatible `/chat/completions` endpoint,
returning a `Completion`. It is the LLM-call substrate the gated Spirit guardian
uses to judge intent — an LLM-API caller, not an agent harness (psyche scope).

A provider is a generic OpenAI-compatible API (endpoint + model + key handle);
adding DeepSeek, MiMo, Kimi, GLM, or MiniMax is configuration through
`meta-signal-agent`, never code. API keys are environment-variable handles
resolved at call time — never hardcoded, never logged.

- `agent-daemon` — the long-lived process. One binary rkyv configuration
  argument; binds an ordinary (working) socket and a `0o600` meta socket.
- `agent` — the thin CLI: one NOTA `signal_agent::Input` argument, `AGENT_SOCKET`
  from the environment, NOTA reply on stdout.

Build offline with the fixture provider (default). The reqwest-backed real call
is behind `--features live-provider`; the live-network test gates on a key.

Read `ARCHITECTURE.md` for the runtime triad and the async provider-call effect,
and `INTENT.md` for the psyche-stated scope.
