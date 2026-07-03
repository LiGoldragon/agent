//! The agent daemon's typed crate error.

use thiserror::Error;
use triad_runtime::{EngineRequestError, FrameError};

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("triad frame: {0}")]
    Frame(#[from] FrameError),

    #[error("ordinary signal frame: {0}")]
    OrdinarySignalFrame(signal_agent::SignalFrameError),

    #[error("meta signal frame: {0}")]
    MetaSignalFrame(meta_signal_agent::SignalFrameError),

    #[error("engine actor: {0}")]
    EngineRequest(#[from] EngineRequestError),

    #[error("configuration read failed: {0}")]
    ConfigurationRead(std::io::Error),

    #[error("configuration write failed: {0}")]
    ConfigurationWrite(std::io::Error),

    #[error("configuration archive decode failed")]
    ConfigurationArchiveDecode,

    #[error("configuration archive encode failed")]
    ConfigurationArchiveEncode,

    #[error("configuration invalid: {0}")]
    ConfigurationInvalid(String),

    #[error("meta request read timed out")]
    MetaRequestReadTimedOut,
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
