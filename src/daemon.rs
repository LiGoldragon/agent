//! The concrete Agent daemon: one process, two authority-bound signal sockets.
//!
//! The runtime binds the canonical ordinary and meta contracts directly. Its
//! only shared mutable noun is the provider engine; there is no generated
//! daemon interface and no runtime-owned schema plane between a contract and
//! its behavior.

use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use meta_signal_agent::{
    z2VNNt, z2VPds, z2VQEC, z2VQtu, z2VT5W, z2VU7B, z2VUiq, z2VVnv, z2VX7d, z2VZMF, z2VaXy, z2VauH,
    z2Vd3L, z2Vdno,
};
use tokio::io::AsyncWriteExt;
use triad_runtime::{
    AcceptedConnection, AsyncListenerSocket, AsyncMultiConnectionRuntime, AsyncMultiListenerDaemon,
    AsyncMultiListenerDaemonError, BindingSurface, ComponentArgument, ComponentCommand, ExitReport,
    FrameBody, LengthPrefixedCodec, MaximumFrameLength, RequestErrorLog, SocketMode,
};

use crate::config::AgentDaemonConfiguration;
use crate::engine::AgentEngine;
use crate::error::{Error, Result};
use crate::provider::Provider;
use crate::registry::ProviderEntry;

const MAXIMUM_FRAME_BYTES: usize = 1024 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Authority {
    Ordinary,
    Meta,
}

impl Display for Authority {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary => formatter.write_str("ordinary"),
            Self::Meta => formatter.write_str("meta"),
        }
    }
}

/// A configured Agent daemon. Construction resolves and validates the single
/// binary configuration argument before any socket or engine is created.
pub struct AgentDaemon {
    configuration: AgentDaemonConfiguration,
}

impl AgentDaemon {
    pub fn from_environment() -> Result<Self> {
        let command = ComponentCommand::from_environment();
        let configuration_path = match command.signal_file_argument()? {
            ComponentArgument::SignalFile(file) => file.into_path(),
            ComponentArgument::InlineDotos(_) | ComponentArgument::DotosFile(_) => {
                return Err(triad_runtime::ArgumentError::ExpectedSignalFile.into());
            }
        };
        Ok(Self {
            configuration: AgentDaemonConfiguration::from_binary_path(configuration_path)?,
        })
    }

    pub fn run_to_exit_code() -> std::process::ExitCode {
        ExitReport::new("agent-daemon").from_result(Self::from_environment().and_then(Self::run))
    }

    pub fn run(self) -> Result<()> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(self.run_async())
    }

    async fn run_async(self) -> Result<()> {
        let configuration = self.configuration;
        let runtime = AgentRuntime::new(Self::build_engine(&configuration));
        let ordinary = AsyncListenerSocket::new(
            Authority::Ordinary,
            configuration.socket_path().to_path_buf(),
        );
        let ordinary = match configuration.socket_mode() {
            Some(mode) => ordinary.with_socket_mode(mode),
            None => ordinary,
        };
        let meta_path = configuration
            .meta_socket_path()
            .ok_or(Error::MissingMetaSocket)?
            .to_path_buf();
        let meta = AsyncListenerSocket::new(Authority::Meta, meta_path)
            .with_socket_mode(SocketMode::new(0o600));

        AsyncMultiListenerDaemon::new(
            [ordinary, meta],
            runtime,
            RequestErrorLog::new("agent-daemon"),
        )
        .with_concurrency_limit(configuration.request_concurrency_limit())
        .run()
        .await
        .map_err(Self::map_daemon_error)
    }

    fn build_engine(configuration: &AgentDaemonConfiguration) -> AgentEngine {
        let mut engine = AgentEngine::with_system_keys(
            crate::registry::ProviderRegistry::new(),
            Self::production_provider(),
        );
        for seed in configuration.bootstrap_providers() {
            engine.configure_provider(seed.clone().into_entry());
        }
        engine
    }

    fn production_provider() -> Box<dyn Provider> {
        #[cfg(feature = "live-provider")]
        {
            Box::new(crate::provider::OpenAiCompatibleProvider::new())
        }
        #[cfg(not(feature = "live-provider"))]
        {
            Box::new(crate::provider::FixtureProvider::new())
        }
    }

    fn map_daemon_error(error: AsyncMultiListenerDaemonError<Error>) -> Error {
        match error {
            AsyncMultiListenerDaemonError::Listener(error) => Error::DaemonListener(error),
            AsyncMultiListenerDaemonError::Start(error)
            | AsyncMultiListenerDaemonError::Stop(error) => error,
        }
    }
}

#[derive(Clone)]
struct AgentRuntime {
    engine: Arc<tokio::sync::Mutex<AgentEngine>>,
    codec: LengthPrefixedCodec,
}

impl AgentRuntime {
    fn new(engine: AgentEngine) -> Self {
        Self {
            engine: Arc::new(tokio::sync::Mutex::new(engine)),
            codec: LengthPrefixedCodec::new(MaximumFrameLength::new(MAXIMUM_FRAME_BYTES)),
        }
    }

    async fn read_frame(&self, connection: &mut AcceptedConnection) -> Result<Vec<u8>> {
        tokio::time::timeout(
            REQUEST_READ_TIMEOUT,
            self.codec.read_body_async(connection.stream_mut()),
        )
        .await
        .map_err(|_| Error::RequestReadTimedOut)?
        .map(|body| body.into_bytes())
        .map_err(Into::into)
    }

    async fn write_frame(&self, connection: &mut AcceptedConnection, bytes: Vec<u8>) -> Result<()> {
        self.codec
            .write_body_async(connection.stream_mut(), &FrameBody::new(bytes))
            .await?;
        connection.stream_mut().flush().await?;
        Ok(())
    }

    async fn serve_ordinary(&self, mut connection: AcceptedConnection) -> Result<()> {
        let frame = self.read_frame(&mut connection).await?;
        let (exchange, input) = signal_agent::ContractMarker::decode_single_request(&frame)?;
        let output = self.engine.lock().await.handle(input).await;
        self.write_frame(&mut connection, output.encode_reply_frame(exchange)?)
            .await
    }

    async fn serve_meta(&self, mut connection: AcceptedConnection) -> Result<()> {
        let frame = self.read_frame(&mut connection).await?;
        let (exchange, input) = meta_signal_agent::ContractMarker::decode_single_request(&frame)?;
        let mut engine = self.engine.lock().await;
        let output = AgentMetaTurn::new(&mut engine).handle(input);
        self.write_frame(&mut connection, output.encode_reply_frame(exchange)?)
            .await
    }
}

impl AsyncMultiConnectionRuntime for AgentRuntime {
    type Listener = Authority;
    type Error = Error;

    async fn handle_connection(
        &self,
        authority: Self::Listener,
        connection: AcceptedConnection,
    ) -> Result<()> {
        match authority {
            Authority::Ordinary => self.serve_ordinary(connection).await,
            Authority::Meta => self.serve_meta(connection).await,
        }
    }
}

struct AgentMetaTurn<'engine> {
    engine: &'engine mut AgentEngine,
}

impl<'engine> AgentMetaTurn<'engine> {
    fn new(engine: &'engine mut AgentEngine) -> Self {
        Self { engine }
    }

    fn handle(self, input: z2VU7B) -> z2VUiq {
        match input {
            z2VU7B::z2VWwB(configure) => self.configure_provider(configure),
            z2VU7B::z2VZYM(retire) => self.retire_provider(retire),
            z2VU7B::z2VYfv(set_default) => self.set_default_provider(set_default),
            z2VU7B::z2Vf8q(_) => z2VUiq::z2VRjY(z2Vd3L::new(z2VZMF::z2VNWD)),
            z2VU7B::z2VXhH(_) => z2VUiq::z2VV4U(z2Vd3L::new(z2VZMF::z2VMBK)),
        }
    }

    fn configure_provider(self, configure: z2VX7d) -> z2VUiq {
        let configuration = configure.into_payload();
        let name = configuration.field_0.payload().clone();
        self.engine.registry_mut().configure(ProviderEntry::new(
            name.clone(),
            configuration.field_1.into_payload(),
            configuration.field_2.into_payload(),
            configuration.field_3.into(),
        ));
        z2VUiq::z2VUhK(z2VQtu::new(z2Vdno::new(name)))
    }

    fn retire_provider(self, retire: z2VauH) -> z2VUiq {
        let name = retire.into_payload();
        if self.engine.registry_mut().retire(name.payload()) {
            z2VUiq::z2VWXh(z2VPds::new(name))
        } else {
            Self::rejected("no such provider")
        }
    }

    fn set_default_provider(self, set_default: z2VNNt) -> z2VUiq {
        let name = set_default.into_payload();
        if self.engine.registry_mut().set_default(name.payload()) {
            z2VUiq::z2VQeG(z2VT5W::new(name))
        } else {
            Self::rejected("no such provider")
        }
    }

    fn rejected(detail: &str) -> z2VUiq {
        z2VUiq::z2Va1P(z2VVnv {
            field_0: z2VQEC::z2VXk7,
            field_1: z2VaXy::new(detail.to_owned()),
        })
    }
}
