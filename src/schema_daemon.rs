//! The agent daemon's hooks — the only daemon code the agent hand-writes.
//!
//! The uniform daemon skeleton (argv parsing, the async working
//! decode -> execute -> encode spine, the two-tier listener bind, and the
//! `ExitReport` entry) is EMITTED into `src/schema/daemon.rs` by
//! schema-rust-next's daemon emitter, driven by the two-tier `NexusDaemonShape`
//! in `build.rs`. The agent fills only the escape hatches through `impl
//! ComponentDaemon for AgentDaemon`: how to load its `Configuration`, how to
//! build its `AgentEngine` (`build_runtime`), how one ordinary `Input` becomes
//! one `Output` (`handle_working_input`), and the meta tier
//! (`handle_meta_connection`).

use std::time::Duration;

use meta_signal_agent::{
    ConfigureProvider, DefaultProviderSet, Lifecycle, LifecycleState, OrderRejection,
    OrderRejectionReason, ProviderConfigured, ProviderRetired, RejectionDetail, RetireProvider,
    SetDefaultProvider, Start, Stop,
};
use signal_agent::schema::lib::{Input, Output};
use tokio::io::AsyncWriteExt;
use triad_runtime::{
    AcceptedConnection, ConnectionContext, FrameBody, LengthPrefixedCodec, MaximumFrameLength,
};

use crate::config::{AgentDaemonConfiguration, ConfigurationError};
use crate::engine::AgentEngine;
use crate::error::Error;
use crate::provider::Provider;
use crate::registry::ProviderEntry;
use crate::schema::daemon::ComponentDaemon;

/// Maximum inbound meta-request-frame body the daemon accepts (1 MiB). A meta
/// request is a few hundred bytes; this bounds a hostile length prefix far below
/// the 4 GiB the u32-prefix codec default would pre-allocate.
const MAXIMUM_META_FRAME_BYTES: usize = 1024 * 1024;

/// How long the meta handler waits for a connected client to send its request
/// frame before dropping the stream. A legitimate client sends immediately.
const META_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The type-level selector for the agent's emitted daemon. It carries no runtime
/// data — it is the marker the emitted `DaemonCommand<AgentDaemon>` and the
/// generated runtime dispatch on, selecting the agent's `Configuration` /
/// `Engine` / `Error` through the `ComponentDaemon` associated types.
pub struct AgentDaemon;

impl AgentDaemon {
    /// The production provider effect: the live OpenAI-compatible reqwest backend
    /// when built with `live-provider`, the deterministic fixture otherwise. The
    /// fixture keeps the default build network-free and the round-trip test
    /// offline; the live backend is selected by feature, not by configuration.
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
}

impl ComponentDaemon for AgentDaemon {
    type Configuration = AgentDaemonConfiguration;
    type ConfigurationError = ConfigurationError;
    type Engine = AgentEngine;
    type Error = Error;

    const PROCESS_NAME: &'static str = "agent-daemon";

    fn load_configuration(
        path: &std::path::Path,
    ) -> Result<Self::Configuration, Self::ConfigurationError> {
        AgentDaemonConfiguration::from_binary_path(path)
    }

    fn build_runtime(configuration: &Self::Configuration) -> Result<Self::Engine, Self::Error> {
        let mut engine = AgentEngine::with_system_keys(
            crate::registry::ProviderRegistry::new(),
            Self::production_provider(),
        );
        for seed in configuration.bootstrap_providers() {
            engine.configure_provider(seed.clone().into_entry());
        }
        Ok(engine)
    }

    async fn handle_working_input(
        engine: &mut Self::Engine,
        input: Input,
        _connection: &ConnectionContext,
    ) -> Result<Output, Self::Error> {
        Ok(engine.handle(input).await)
    }

    /// Serve one meta request end to end: decode a `meta-signal-agent` `Input`
    /// off the length-prefixed frame, mutate the provider registry, and write the
    /// meta `Output` back. The meta wire codec is component-owned (the emitter
    /// routes the meta socket here), so this owns the full read/handle/write.
    async fn handle_meta_connection(
        engine: &mut Self::Engine,
        mut connection: AcceptedConnection,
    ) -> Result<(), Self::Error> {
        let codec = LengthPrefixedCodec::new(MaximumFrameLength::new(MAXIMUM_META_FRAME_BYTES));
        let body = tokio::time::timeout(
            META_REQUEST_READ_TIMEOUT,
            codec.read_body_async(connection.stream_mut()),
        )
        .await
        .map_err(|_| Error::MetaRequestReadTimedOut)??;
        let (_route, input) = meta_signal_agent::Input::decode_signal_frame(body.bytes())?;
        let reply = AgentMetaHandler::new(engine).handle(input);
        codec
            .write_body_async(
                connection.stream_mut(),
                &FrameBody::new(reply.encode_signal_frame()?),
            )
            .await?;
        connection.stream_mut().flush().await?;
        Ok(())
    }
}

/// The meta-tier projection: it owns the lowering from a `meta-signal-agent`
/// operation to a provider-registry mutation and the typed reply. It borrows the
/// engine for the duration of one meta request.
struct AgentMetaHandler<'engine> {
    engine: &'engine mut AgentEngine,
}

impl<'engine> AgentMetaHandler<'engine> {
    fn new(engine: &'engine mut AgentEngine) -> Self {
        Self { engine }
    }

    fn handle(self, input: meta_signal_agent::Input) -> meta_signal_agent::Output {
        match input {
            meta_signal_agent::Input::ConfigureProvider(configure) => {
                self.configure_provider(configure)
            }
            meta_signal_agent::Input::RetireProvider(retire) => self.retire_provider(retire),
            meta_signal_agent::Input::SetDefaultProvider(set_default) => {
                self.set_default_provider(set_default)
            }
            meta_signal_agent::Input::Start(start) => Self::started(start),
            meta_signal_agent::Input::Stop(stop) => Self::stopped(stop),
        }
    }

    fn configure_provider(self, configure: ConfigureProvider) -> meta_signal_agent::Output {
        let configuration = configure.into_payload();
        let name = configuration.name.payload().clone();
        self.engine.registry_mut().configure(ProviderEntry::new(
            name.clone(),
            configuration.endpoint.into_payload(),
            configuration.default_model.into_payload(),
            configuration.secret_source.into(),
        ));
        meta_signal_agent::Output::ProviderConfigured(ProviderConfigured::new(
            meta_signal_agent::ProviderName::new(name),
        ))
    }

    fn retire_provider(self, retire: RetireProvider) -> meta_signal_agent::Output {
        let name = retire.into_payload();
        if self.engine.registry_mut().retire(name.payload()) {
            meta_signal_agent::Output::ProviderRetired(ProviderRetired::new(name))
        } else {
            Self::rejected(OrderRejectionReason::ProviderUnknown, "no such provider")
        }
    }

    fn set_default_provider(self, set_default: SetDefaultProvider) -> meta_signal_agent::Output {
        let name = set_default.into_payload();
        if self.engine.registry_mut().set_default(name.payload()) {
            meta_signal_agent::Output::DefaultProviderSet(DefaultProviderSet::new(name))
        } else {
            Self::rejected(OrderRejectionReason::ProviderUnknown, "no such provider")
        }
    }

    fn started(_start: Start) -> meta_signal_agent::Output {
        meta_signal_agent::Output::Started(Lifecycle::new(LifecycleState::Started))
    }

    fn stopped(_stop: Stop) -> meta_signal_agent::Output {
        meta_signal_agent::Output::Stopped(Lifecycle::new(LifecycleState::Stopped))
    }

    fn rejected(reason: OrderRejectionReason, detail: &str) -> meta_signal_agent::Output {
        meta_signal_agent::Output::OrderRejected(OrderRejection {
            reason,
            detail: RejectionDetail::new(detail.to_owned()),
        })
    }
}
