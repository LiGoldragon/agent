//! The agent CLI's daemon client — the thin text-to-Signal adapter.
//!
//! The CLI is the daemon's first client, not a triad leg, and is
//! eventually-obsolete machinery once peers speak Signal directly. It reads one
//! DOTOS `signal_agent::Input` off argv, encodes it to a binary signal frame on
//! the daemon's ordinary socket, decodes the binary reply, and renders it back
//! as DOTOS. The daemon never sees DOTOS — only the binary frame the CLI translated.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use signal_agent::{DotosEncode, FrameBody as SignalFrameBody, Input, Output};
use signal_frame::{ExchangeIdentifier, ExchangeLane, LaneSequence, Reply, SessionEpoch, SubReply};
use triad_runtime::{FrameBody, LengthPrefixedCodec};

use crate::error::{Error, Result};

/// The ordinary agent socket, resolved from the environment (no flag, no socket
/// argument). Mirrors the spirit/message CLI convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSocket {
    path: PathBuf,
}

impl AgentSocket {
    pub fn from_environment() -> Option<Self> {
        std::env::var_os("AGENT_SOCKET").map(Self::from_path)
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn client(&self) -> AgentClient {
        AgentClient::from_socket(self.clone())
    }
}

/// A one-shot client over the ordinary agent socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentClient {
    socket: AgentSocket,
    codec: LengthPrefixedCodec,
}

impl AgentClient {
    pub fn from_socket(socket: AgentSocket) -> Self {
        Self {
            socket,
            codec: LengthPrefixedCodec::default(),
        }
    }

    pub fn call(&self, input: Input) -> Result<Output> {
        let mut stream = UnixStream::connect(self.socket.path())?;
        let exchange = ExchangeIdentifier::new(
            SessionEpoch::new(0),
            ExchangeLane::Connector,
            LaneSequence::first(),
        );
        let request = FrameBody::new(input.encode_request_frame(exchange)?);
        self.codec.write_body(&mut stream, &request)?;
        stream.flush()?;
        let reply = self.codec.read_body(&mut stream)?;
        let frame = signal_agent::ContractMarker::decode_frame(&reply.into_bytes())?;
        match frame.into_body() {
            SignalFrameBody::Reply {
                exchange: received,
                reply,
            } if received == exchange => match reply {
                Reply::Accepted { per_operation, .. } => match per_operation.into_head() {
                    SubReply::Ok(output) => Ok(output),
                    _ => Err(Error::Io(std::io::Error::other(
                        "agent request did not commit",
                    ))),
                },
                Reply::Rejected { .. } => Err(Error::Io(std::io::Error::other(
                    "agent request was rejected",
                ))),
            },
            SignalFrameBody::Reply { .. } => Err(Error::Io(std::io::Error::other(
                "agent reply exchange mismatch",
            ))),
            _ => Err(Error::Io(std::io::Error::other(
                "agent reply was not a reply frame",
            ))),
        }
    }
}

/// The CLI command: one DOTOS argument naming a `signal_agent::Input` request.
pub struct CommandLine {
    argument: Option<String>,
}

impl CommandLine {
    pub fn from_environment() -> Self {
        Self {
            argument: std::env::args().nth(1),
        }
    }

    pub fn run(self, mut output: impl Write) -> Result<()> {
        let argument = self
            .argument
            .ok_or_else(|| Error::Io(std::io::Error::other("missing DOTOS request argument")))?;
        let input: Input = argument
            .parse()
            .map_err(|error| Error::Io(std::io::Error::other(format!("invalid DOTOS: {error}"))))?;
        let socket = AgentSocket::from_environment()
            .ok_or_else(|| Error::Io(std::io::Error::other("AGENT_SOCKET is not set")))?;
        let reply = socket.client().call(input)?;
        writeln!(output, "{}", reply.to_dotos()).map_err(Error::Io)?;
        Ok(())
    }
}
