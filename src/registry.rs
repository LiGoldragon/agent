//! The provider registry — the daemon's policy state.
//!
//! A `ProviderEntry` is the durable shape `meta-signal-agent`'s
//! `ProviderConfiguration` carries: name, endpoint, default model, and an
//! API-key HANDLE (an environment-variable name). The registry resolves a
//! `ProviderName` plus a `Prompt` into a fully-resolved `ProviderCall`, reading
//! the secret from the environment at call time. The secret value never lives in
//! the registry, only the handle.

use signal_agent::{ChatMessage, Prompt};

use crate::provider::{ProviderApiKey, ProviderCall, ProviderMessage};

/// One configured provider: the OpenAI-compatible facts plus the key handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEntry {
    name: String,
    endpoint: String,
    default_model: String,
    api_key_handle: String,
}

impl ProviderEntry {
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        default_model: impl Into<String>,
        api_key_handle: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            endpoint: endpoint.into(),
            default_model: default_model.into(),
            api_key_handle: api_key_handle.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    pub fn api_key_handle(&self) -> &str {
        &self.api_key_handle
    }
}

/// Why a `ProviderCall` could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NoProviderConfigured,
    ProviderUnknown(String),
    KeyHandleUnset(String),
}

/// The set of configured providers plus the default. Held in the engine;
/// configured through the meta tier. The durable-redb projection is deferred
/// (see `schema/sema.schema`).
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    entries: Vec<ProviderEntry>,
    default_provider: Option<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a provider by name. The first provider configured
    /// becomes the default when none is set.
    pub fn configure(&mut self, entry: ProviderEntry) {
        if self.default_provider.is_none() {
            self.default_provider = Some(entry.name().to_owned());
        }
        match self
            .entries
            .iter_mut()
            .find(|existing| existing.name() == entry.name())
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    /// Remove a provider by name. Returns whether it was present. Clears the
    /// default when the retired provider was the default.
    pub fn retire(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.name() != name);
        if self.default_provider.as_deref() == Some(name) {
            self.default_provider = self.entries.first().map(|entry| entry.name().to_owned());
        }
        self.entries.len() != before
    }

    /// Set the default provider. Returns whether the named provider exists.
    pub fn set_default(&mut self, name: &str) -> bool {
        if self.entries.iter().any(|entry| entry.name() == name) {
            self.default_provider = Some(name.to_owned());
            true
        } else {
            false
        }
    }

    pub fn default_provider(&self) -> Option<&str> {
        self.default_provider.as_deref()
    }

    fn entry(&self, name: &str) -> Option<&ProviderEntry> {
        self.entries.iter().find(|entry| entry.name() == name)
    }

    /// Resolve a `Prompt` into a `ProviderCall`: pick the provider (the prompt's
    /// named provider, else the registry default), pick the model (the prompt's
    /// model, else the provider's default), resolve the key handle from the
    /// environment, and project the chat transcript. The key resolution is the
    /// `KeySource`'s job so tests can inject a key without touching process env.
    pub fn resolve(
        &self,
        prompt: &Prompt,
        keys: &dyn KeySource,
    ) -> Result<ProviderCall, ResolveError> {
        let provider_name = prompt
            .options
            .provider
            .as_ref()
            .map(|provider| provider.payload().clone())
            .or_else(|| self.default_provider.clone())
            .ok_or(ResolveError::NoProviderConfigured)?;
        let entry = self
            .entry(&provider_name)
            .ok_or_else(|| ResolveError::ProviderUnknown(provider_name.clone()))?;
        let model = prompt
            .options
            .model
            .as_ref()
            .map(|model| model.payload().clone())
            .unwrap_or_else(|| entry.default_model().to_owned());
        let api_key = keys
            .resolve(entry.api_key_handle())
            .ok_or_else(|| ResolveError::KeyHandleUnset(entry.api_key_handle().to_owned()))?;
        Ok(ProviderCall::new(
            entry.endpoint().to_owned(),
            model,
            api_key,
            prompt
                .system
                .as_ref()
                .map(|system| system.payload().clone()),
            prompt
                .transcript
                .payload()
                .iter()
                .map(Self::project_message)
                .collect(),
            prompt.options.output_mode,
            prompt
                .options
                .temperature_milli
                .as_ref()
                .map(|temperature| *temperature.payload()),
            prompt
                .options
                .maximum_output_tokens
                .as_ref()
                .map(|maximum| *maximum.payload()),
        ))
    }

    fn project_message(message: &ChatMessage) -> ProviderMessage {
        ProviderMessage::new(message.role, message.text.payload().clone())
    }
}

/// Where the secret value for a key handle comes from. The production source
/// reads the named environment variable; tests inject a literal so a fixture
/// call needs no real key in the process environment.
pub trait KeySource {
    fn resolve(&self, handle: &str) -> Option<ProviderApiKey>;
}

/// The production key source: resolve the handle as an environment-variable
/// name. The secret value is read here and flows only into the `ProviderCall`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvironmentKeySource;

impl KeySource for EnvironmentKeySource {
    fn resolve(&self, handle: &str) -> Option<ProviderApiKey> {
        std::env::var(handle).ok().map(ProviderApiKey::new)
    }
}
