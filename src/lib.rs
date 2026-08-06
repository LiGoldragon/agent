//! `agent` runtime — the LLM-API-call daemon.
//!
//! `agent` makes provider HTTP API calls in an OpenAI-compatible
//! chat-completions style (psyche scope Spirit `iucr`, `f8k7`): it is the
//! LLM-call substrate the gated Spirit guardian uses to judge intent, NOT an
//! agent-harness front door. Harness backends are deferred.
//!
//! Its ordinary and policy surfaces are the authority-sealed `signal-agent`
//! and `meta-signal-agent` contracts. The concrete daemon binds those two
//! authorities directly; this runtime owns no second schema language or
//! generated interface.
//!
//! The one external effect is the provider call: a decoded `Call(Prompt)` lowers
//! to its one asynchronous provider effect, which makes the OpenAI-compatible
//! `/chat/completions` HTTPS request. The provider is
//! resolved from the registry by name; the API key is resolved from a typed
//! secret-source backend at call time, never hardcoded.

pub mod client;
pub mod config;
pub mod daemon;
pub mod engine;
pub mod error;
pub mod provider;
pub mod registry;

pub use config::{AgentDaemonConfiguration, ConfigurationError, ProviderSeed};
pub use daemon::AgentDaemon;
pub use engine::AgentEngine;
pub use error::{Error, Result};
pub use provider::{
    FixtureProvider, Provider, ProviderAuthorization, ProviderCall, ProviderCompletion,
    ProviderFailure,
};
pub use registry::{ProviderEntry, ProviderRegistry, SecretSource};
