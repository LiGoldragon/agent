//! `agent` runtime — the LLM-API-call daemon.
//!
//! `agent` makes provider HTTP API calls in an OpenAI-compatible
//! chat-completions style (psyche scope Spirit `iucr`, `f8k7`): it is the
//! LLM-call substrate the gated Spirit guardian uses to judge intent, NOT an
//! agent-harness front door. Harness backends are deferred.
//!
//! It is a schema-derived triad component on the emitted daemon runtime. The
//! two plane schemas (`schema/nexus.schema`, `schema/sema.schema`) plus the
//! daemon module generate the checked-in modules under `src/schema/` through
//! `schema-rust`; the working tier's `Input`/`Output` come from the
//! dependency contract `signal-agent`. The hand-written code here is the thin
//! runtime around those generated interfaces.
//!
//! The one external effect is the provider call: a decoded `Call(Prompt)` lowers
//! to a Nexus `CallProvider` effect, and the generated async runner awaits
//! `run_effect`, which makes the OpenAI-compatible `/chat/completions` HTTPS
//! request off the engine mailbox (no blocking in the handler). The provider is
//! resolved from the registry by name; the API key is resolved from a typed
//! secret-source backend at call time, never hardcoded.

pub mod client;
pub mod config;
pub mod engine;
pub mod error;
pub mod provider;
pub mod registry;
pub mod schema_daemon;

pub mod schema {
    #[rustfmt::skip]
    pub mod nexus;
    #[rustfmt::skip]
    pub mod sema;
    #[rustfmt::skip]
    pub mod daemon;
}

pub use config::{AgentDaemonConfiguration, ConfigurationError, ProviderSeed};
pub use engine::AgentEngine;
pub use error::{Error, Result};
pub use provider::{
    FixtureProvider, Provider, ProviderAuthorization, ProviderCall, ProviderCompletion,
    ProviderFailure,
};
pub use registry::{ProviderEntry, ProviderRegistry, SecretSource};
pub use schema::daemon::{ComponentDaemon, DaemonCommand, DaemonEntry, DaemonError};
pub use schema_daemon::AgentDaemon;
