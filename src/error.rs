//! The agent daemon's typed crate error.

use thiserror::Error;
use triad_runtime::{ArgumentError, AsyncListenerError, FrameError};

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("daemon argument: {0}")]
    Argument(#[from] ArgumentError),

    #[error("daemon listener: {0}")]
    DaemonListener(AsyncListenerError),

    #[error("daemon meta socket path missing from configuration")]
    MissingMetaSocket,

    #[error("request read timed out")]
    RequestReadTimedOut,

    #[error("triad frame: {0}")]
    Frame(#[from] FrameError),

    #[error("ordinary signal frame: {0}")]
    OrdinarySignalFrame(signal_agent::SignalFrameError),

    #[error("meta signal frame: {0}")]
    MetaSignalFrame(meta_signal_agent::SignalFrameError),

    #[error("configuration read failed: {0}")]
    ConfigurationRead(std::io::Error),

    #[error("configuration write failed: {0}")]
    ConfigurationWrite(std::io::Error),

    #[error("configuration archive decode failed")]
    ConfigurationArchiveDecode,

    #[error("configuration archive encode failed")]
    ConfigurationArchiveEncode,

    #[error("configuration: {0}")]
    Configuration(#[from] crate::config::ConfigurationError),
}

impl From<signal_agent::SignalFrameError> for Error {
    fn from(error: signal_agent::SignalFrameError) -> Self {
        Self::OrdinarySignalFrame(error)
    }
}

impl From<meta_signal_agent::SignalFrameError> for Error {
    fn from(error: meta_signal_agent::SignalFrameError) -> Self {
        Self::MetaSignalFrame(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
