//! The agent daemon's binary rkyv startup configuration.
//!
//! Loaded as a binary rkyv file from the single argv argument (the
//! single-argument rule). The daemon never parses NOTA, configuration included;
//! a deploy/bootstrap tool encodes typed NOTA into this binary form before it
//! reaches the daemon.
//!
//! The configuration carries the two socket paths (ordinary + meta) and the
//! durable database path the uniform `DaemonConfiguration` trait requires. The
//! provider registry is populated through the meta tier after startup, never
//! from a flag and never from inline NOTA. An optional `bootstrap_providers`
//! seed lets a deploy tool ship the daemon a starting registry in the binary
//! startup message — still binary, still typed, no NOTA parsed by the daemon.

use std::path::Path;

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use thiserror::Error;
use triad_runtime::{DaemonConfiguration, SocketMode};

use crate::registry::ProviderEntry;

/// A provider seed carried in the binary startup message: the same four facts
/// `meta-signal-agent`'s `ProviderConfiguration` carries (name, endpoint,
/// default model, key handle). The key handle is an environment-variable name;
/// the secret value is never in the configuration.
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderSeed {
    pub name: String,
    pub endpoint: String,
    pub default_model: String,
    pub api_key_handle: String,
}

impl ProviderSeed {
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

    pub fn into_entry(self) -> ProviderEntry {
        ProviderEntry::new(
            self.name,
            self.endpoint,
            self.default_model,
            self.api_key_handle,
        )
    }
}

/// Binary rkyv startup configuration for `agent-daemon`.
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentDaemonConfiguration {
    ordinary_socket_path: String,
    meta_socket_path: String,
    meta_socket_mode: u32,
    database_path: String,
    bootstrap_providers: Vec<ProviderSeed>,
}

impl AgentDaemonConfiguration {
    pub fn new(
        ordinary_socket_path: impl Into<String>,
        meta_socket_path: impl Into<String>,
        meta_socket_mode: u32,
        database_path: impl Into<String>,
        bootstrap_providers: Vec<ProviderSeed>,
    ) -> Self {
        Self {
            ordinary_socket_path: ordinary_socket_path.into(),
            meta_socket_path: meta_socket_path.into(),
            meta_socket_mode,
            database_path: database_path.into(),
            bootstrap_providers,
        }
    }

    pub fn bootstrap_providers(&self) -> &[ProviderSeed] {
        &self.bootstrap_providers
    }

    pub fn from_binary_path(path: impl AsRef<Path>) -> Result<Self, ConfigurationError> {
        let bytes = std::fs::read(path).map_err(ConfigurationError::Read)?;
        Self::from_binary_bytes(&bytes)
    }

    pub fn from_binary_bytes(bytes: &[u8]) -> Result<Self, ConfigurationError> {
        rkyv::from_bytes::<Self, rkyv::rancor::Error>(bytes)
            .map_err(|_| ConfigurationError::ArchiveDecode)
    }

    pub fn to_binary_bytes(&self) -> Result<Vec<u8>, ConfigurationError> {
        rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map(|bytes| bytes.to_vec())
            .map_err(|_| ConfigurationError::ArchiveEncode)
    }

    pub fn write_binary_file(&self, path: impl AsRef<Path>) -> Result<(), ConfigurationError> {
        std::fs::write(path, self.to_binary_bytes()?).map_err(ConfigurationError::Write)
    }
}

impl DaemonConfiguration for AgentDaemonConfiguration {
    fn socket_path(&self) -> &Path {
        Path::new(&self.ordinary_socket_path)
    }

    fn meta_socket_path(&self) -> Option<&Path> {
        Some(Path::new(&self.meta_socket_path))
    }

    fn meta_socket_mode(&self) -> Option<SocketMode> {
        Some(SocketMode::new(self.meta_socket_mode))
    }

    /// The agent daemon holds its provider registry in the engine (the durable
    /// redb projection is deferred), so this names an unused-on-the-call-path
    /// location kept only to satisfy the uniform trait surface.
    fn database_path(&self) -> &Path {
        Path::new(&self.database_path)
    }
}

#[derive(Debug, Error)]
pub enum ConfigurationError {
    #[error("failed to read binary configuration: {0}")]
    Read(std::io::Error),

    #[error("failed to write binary configuration: {0}")]
    Write(std::io::Error),

    #[error("failed to encode binary configuration")]
    ArchiveEncode,

    #[error("failed to decode binary configuration")]
    ArchiveDecode,
}
