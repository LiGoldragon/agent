//! The provider registry — the daemon's policy state.
//!
//! A `ProviderEntry` is the durable shape `meta-signal-agent`'s
//! `ProviderConfiguration` carries: name, endpoint, default model, and a typed
//! secret-source reference. The registry resolves a `ProviderName` plus a
//! `Prompt` into a fully-resolved `ProviderCall`, asking the daemon-owned
//! `KeySource` to resolve configured secret-bearing backends. The secret value
//! never lives in the registry, only the source reference.

use std::path::Path;

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use signal_agent::{ChatMessage, Prompt};
use thiserror::Error;

use crate::provider::{ProviderApiKey, ProviderAuthorization, ProviderCall, ProviderMessage};

/// One configured provider: the OpenAI-compatible facts plus the secret source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEntry {
    name: String,
    endpoint: String,
    default_model: String,
    secret_source: SecretSource,
}

impl ProviderEntry {
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        default_model: impl Into<String>,
        secret_source: SecretSource,
    ) -> Self {
        Self {
            name: name.into(),
            endpoint: endpoint.into(),
            default_model: default_model.into(),
            secret_source,
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

    pub fn secret_source(&self) -> &SecretSource {
        &self.secret_source
    }
}

/// A typed reference to where a provider API key lives. This is configuration,
/// not a secret value. `NoSecret` sends no Authorization header.
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum SecretSource {
    Environment(EnvironmentVariable),
    Gopass(GopassPath),
    File(SecretFilePath),
    NoSecret,
}

impl SecretSource {
    pub fn environment(variable: impl Into<String>) -> Self {
        Self::Environment(EnvironmentVariable::new(variable))
    }

    pub fn gopass(path: impl Into<String>) -> Self {
        Self::Gopass(GopassPath::new(path))
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self::File(SecretFilePath::new(path))
    }

    pub fn no_secret() -> Self {
        Self::NoSecret
    }

    fn description(&self) -> String {
        match self {
            Self::Environment(variable) => format!("environment:{}", variable.as_str()),
            Self::Gopass(path) => format!("gopass:{}", path.as_str()),
            Self::File(path) => format!("file:{}", path.as_str()),
            Self::NoSecret => "no-secret".to_owned(),
        }
    }

    fn requires_secret_resolution(&self) -> bool {
        !matches!(self, Self::NoSecret)
    }
}

impl From<meta_signal_agent::SecretSource> for SecretSource {
    fn from(source: meta_signal_agent::SecretSource) -> Self {
        match source {
            meta_signal_agent::SecretSource::Environment(secret) => {
                Self::environment(secret.into_payload().into_payload())
            }
            meta_signal_agent::SecretSource::Gopass(secret) => {
                Self::gopass(secret.into_payload().into_payload())
            }
            meta_signal_agent::SecretSource::File(secret) => {
                Self::file(secret.into_payload().into_payload())
            }
            meta_signal_agent::SecretSource::NoSecret => Self::no_secret(),
        }
    }
}

#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentVariable(String);

impl EnvironmentVariable {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct GopassPath(String);

impl GopassPath {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SecretFilePath(String);

impl SecretFilePath {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

/// Why a `ProviderCall` could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NoProviderConfigured,
    ProviderUnknown(String),
    SecretUnavailable(KeyResolutionError),
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
    /// model, else the provider's default), resolve the configured secret source,
    /// and project the chat transcript.
    pub async fn resolve(
        &self,
        prompt: &Prompt,
        keys: &(dyn KeySource + Send + Sync),
    ) -> Result<ProviderCall, ResolveError> {
        let options = prompt.prompt_options();
        let provider_name = prompt
            .prompt_options()
            .provider()
            .map(|provider| provider.payload().clone())
            .or_else(|| self.default_provider.clone())
            .ok_or(ResolveError::NoProviderConfigured)?;
        let entry = self
            .entry(&provider_name)
            .ok_or_else(|| ResolveError::ProviderUnknown(provider_name.clone()))?;
        let model = options
            .model()
            .as_ref()
            .map(|model| model.payload().clone())
            .unwrap_or_else(|| entry.default_model().to_owned());
        let authorization = if entry.secret_source().requires_secret_resolution() {
            ProviderAuthorization::bearer(
                keys.resolve(entry.secret_source().clone())
                    .await
                    .map_err(ResolveError::SecretUnavailable)?,
            )
        } else {
            ProviderAuthorization::none()
        };
        Ok(ProviderCall::new(
            entry.endpoint().to_owned(),
            model,
            authorization,
            prompt.system().map(|system| system.payload().clone()),
            prompt
                .chat_transcript()
                .payload()
                .iter()
                .map(Self::project_message)
                .collect(),
            options.output_mode(),
            options
                .temperature_milli()
                .map(|temperature| *temperature.payload()),
            options
                .maximum_output_tokens()
                .map(|maximum| *maximum.payload()),
            options.reasoning_effort().copied(),
            options.thinking_mode().copied(),
        ))
    }

    fn project_message(message: &ChatMessage) -> ProviderMessage {
        ProviderMessage::new(message.chat_role, message.user_text.payload().clone())
    }
}

pub type KeySourceFuture<'source> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<ProviderApiKey, KeyResolutionError>>
            + Send
            + 'source,
    >,
>;

/// Where the secret value for a configured source comes from. The production
/// source supports environment variables, gopass, and secret files; tests inject
/// a literal so a fixture call needs no real key in the process environment.
pub trait KeySource {
    fn resolve(&self, source: SecretSource) -> KeySourceFuture<'_>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyResolutionError {
    #[error("environment variable is unset: {0}")]
    EnvironmentUnset(String),

    #[error("gopass secret unavailable at {path}: {detail}")]
    GopassUnavailable { path: String, detail: String },

    #[error("secret file unavailable at {path}: {detail}")]
    FileUnavailable { path: String, detail: String },

    #[error("secret source returned an empty secret: {0}")]
    EmptySecret(String),
}

/// The production key source. The secret value is read here and flows only into
/// the `ProviderCall`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemKeySource;

impl KeySource for SystemKeySource {
    fn resolve(&self, source: SecretSource) -> KeySourceFuture<'_> {
        Box::pin(async move {
            let description = source.description();
            let value = match source {
                SecretSource::Environment(variable) => {
                    std::env::var(variable.as_str()).map_err(|_| {
                        KeyResolutionError::EnvironmentUnset(variable.as_str().to_owned())
                    })?
                }
                SecretSource::Gopass(path) => Self::resolve_gopass(path).await?,
                SecretSource::File(path) => Self::resolve_file(path).await?,
                SecretSource::NoSecret => return Ok(ProviderApiKey::new("")),
            };
            let api_key = ProviderApiKey::from_secret_output(value);
            if api_key.is_empty() {
                Err(KeyResolutionError::EmptySecret(description))
            } else {
                Ok(api_key)
            }
        })
    }
}

impl SystemKeySource {
    async fn resolve_gopass(path: GopassPath) -> Result<String, KeyResolutionError> {
        let output = tokio::process::Command::new("gopass")
            .arg("show")
            .arg("-o")
            .arg(path.as_str())
            .output()
            .await
            .map_err(|error| KeyResolutionError::GopassUnavailable {
                path: path.as_str().to_owned(),
                detail: error.to_string(),
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(KeyResolutionError::GopassUnavailable {
                path: path.as_str().to_owned(),
                detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    async fn resolve_file(path: SecretFilePath) -> Result<String, KeyResolutionError> {
        tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|error| KeyResolutionError::FileUnavailable {
                path: path.as_str().to_owned(),
                detail: error.to_string(),
            })
    }
}
