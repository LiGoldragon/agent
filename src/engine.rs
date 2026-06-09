//! The agent runtime engine — the data-bearing noun the three schema-emitted
//! planes attach to.
//!
//! `AgentEngine` owns the component state: the provider registry (policy state),
//! the `Provider` effect implementation (fixture or the live OpenAI-compatible
//! call), and the key source. It implements the generated `NexusEngine` and
//! `SemaEngine` traits, and owns the meta-tier projection.
//!
//! The call flow (record-970 forward-with-effect shape): a decoded `Call(Prompt)`
//! becomes `NexusWork::SignalArrived`; `decide` emits
//! `CommandEffect(CallProvider(Prompt))`; the generated async runner awaits
//! `run_effect`, which resolves the registry and asks the `Provider` to complete
//! the call — the only place the daemon touches the network, and it does so off
//! the engine mailbox through the async effect seam, never blocking a handler.

use signal_agent::{
    CallRejection, CallRejectionReason, Completion, CompletionText, CompletionTokenCount,
    OperationKind, Output, PromptTokenCount, RejectionDetail, RequestUnimplemented, StopReasonText,
    TokenUsage, UnimplementedReason,
};

use crate::provider::{Provider, ProviderCall, ProviderCompletion, ProviderFailure};
use crate::registry::{
    EnvironmentKeySource, KeySource, ProviderEntry, ProviderRegistry, ResolveError,
};
use crate::schema::nexus::{
    self as nexus_schema, CommandEffect, EffectCompleted, NexusAction, NexusEngine, NexusWork,
    ProviderCallCommand, ProviderOutcome,
};
use crate::schema::sema::{
    self as sema_schema, ReadInput as SemaReadInput, ReadOutput as SemaReadOutput, SemaEngine,
    Stateless, WriteInput as SemaWriteInput, WriteOutput as SemaWriteOutput,
};

/// The agent daemon's engine. `Provider` is boxed so the fixture and the live
/// reqwest backend share one engine type; `KeySource` is boxed so tests inject
/// a key without touching process environment.
pub struct AgentEngine {
    registry: ProviderRegistry,
    provider: Box<dyn Provider>,
    keys: Box<dyn KeySource + Send + Sync>,
}

impl std::fmt::Debug for AgentEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentEngine")
            .field("registry", &self.registry)
            .field("provider", &"<provider>")
            .finish()
    }
}

impl AgentEngine {
    pub fn new(
        registry: ProviderRegistry,
        provider: Box<dyn Provider>,
        keys: Box<dyn KeySource + Send + Sync>,
    ) -> Self {
        Self {
            registry,
            provider,
            keys,
        }
    }

    /// The production engine: the live OpenAI-compatible provider when built with
    /// `live-provider`, the fixture otherwise, plus the environment key source.
    pub fn with_environment_keys(registry: ProviderRegistry, provider: Box<dyn Provider>) -> Self {
        Self::new(registry, provider, Box::new(EnvironmentKeySource))
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut ProviderRegistry {
        &mut self.registry
    }

    /// Seed the registry from binary startup configuration. Used at startup; the
    /// meta tier mutates the registry thereafter.
    pub fn configure_provider(&mut self, entry: ProviderEntry) {
        self.registry.configure(entry);
    }

    /// Run one decoded ordinary `Input` end to end and return the `Output`. The
    /// generated `NexusEngine::execute` owns the recursive runner loop; the
    /// engine supplies the decision and the async provider effect.
    pub async fn handle(&mut self, input: signal_agent::Input) -> Output {
        let work = NexusWork::signal_arrived(input).with_origin_route(FORWARD_ORIGIN_ROUTE);
        let action = self.execute(work).await.into_root();
        match action {
            NexusAction::ReplyToSignal(output) => output,
            other => Output::CallRejected(CallRejection {
                reason: CallRejectionReason::ProviderRejected,
                detail: RejectionDetail::new(format!(
                    "nexus runner returned non-reply action: {other:?}"
                )),
            }),
        }
    }

    /// The decision for an arrived ordinary `Input`.
    fn decide_signal(&self, input: signal_agent::Input) -> NexusAction {
        match input {
            signal_agent::Input::Call(call) => {
                NexusAction::CommandEffect(ProviderCallCommand::call_provider(call.into_payload()))
            }
            signal_agent::Input::StreamCall(_) => {
                NexusAction::ReplyToSignal(Output::RequestUnimplemented(RequestUnimplemented {
                    operation: OperationKind::StreamCall,
                    reason: UnimplementedReason::NotInPrototypeScope,
                }))
            }
            signal_agent::Input::CancelStream(_) => {
                NexusAction::ReplyToSignal(Output::RequestUnimplemented(RequestUnimplemented {
                    operation: OperationKind::CancelStream,
                    reason: UnimplementedReason::NotInPrototypeScope,
                }))
            }
        }
    }

    /// Turn a completed provider call into the Signal `Output` to reply with.
    fn decide_effect_completed(&self, outcome: ProviderOutcome) -> NexusAction {
        match outcome {
            ProviderOutcome::Completed(completion) => {
                NexusAction::ReplyToSignal(Output::Completed(completion))
            }
            ProviderOutcome::Rejected(rejection) => {
                NexusAction::ReplyToSignal(Output::CallRejected(rejection))
            }
        }
    }

    /// Run the one effect the agent declares: call the configured provider. This
    /// resolves the prompt against the registry (provider, model, key handle),
    /// makes the OpenAI-compatible call through the `Provider`, and lifts the
    /// result into a typed `ProviderOutcome`.
    async fn run_provider_effect(&self, command: CommandEffect) -> ProviderOutcome {
        let ProviderCallCommand::CallProvider(prompt) = command;
        match self.registry.resolve(&prompt, self.keys.as_ref()) {
            Ok(call) => self.complete_call(call).await,
            Err(error) => ProviderOutcome::Rejected(Self::resolve_rejection(error)),
        }
    }

    async fn complete_call(&self, call: ProviderCall) -> ProviderOutcome {
        match self.provider.complete(call).await {
            Ok(completion) => ProviderOutcome::Completed(Self::completion(completion)),
            Err(failure) => ProviderOutcome::Rejected(Self::failure_rejection(failure)),
        }
    }

    fn completion(completion: ProviderCompletion) -> Completion {
        Completion {
            text: CompletionText::new(completion.text),
            stop_reason: StopReasonText::new(completion.stop_reason),
            usage: TokenUsage {
                prompt_tokens: completion.prompt_tokens.map(PromptTokenCount::new),
                completion_tokens: completion.completion_tokens.map(CompletionTokenCount::new),
            },
        }
    }

    fn resolve_rejection(error: ResolveError) -> CallRejection {
        match error {
            ResolveError::NoProviderConfigured => CallRejection {
                reason: CallRejectionReason::NoProviderConfigured,
                detail: RejectionDetail::new(
                    "no provider configured and prompt named none".to_owned(),
                ),
            },
            ResolveError::ProviderUnknown(name) => CallRejection {
                reason: CallRejectionReason::NoProviderConfigured,
                detail: RejectionDetail::new(format!("provider not in registry: {name}")),
            },
            ResolveError::KeyHandleUnset(handle) => CallRejection {
                reason: CallRejectionReason::DaemonUnconfigured,
                detail: RejectionDetail::new(format!("api key handle unset: {handle}")),
            },
        }
    }

    fn failure_rejection(failure: ProviderFailure) -> CallRejection {
        match failure {
            ProviderFailure::Unreachable(detail) => CallRejection {
                reason: CallRejectionReason::ProviderUnreachable,
                detail: RejectionDetail::new(detail),
            },
            ProviderFailure::ProviderRejected(detail) => CallRejection {
                reason: CallRejectionReason::ProviderRejected,
                detail: RejectionDetail::new(detail),
            },
            ProviderFailure::OutputModeUnsupported => CallRejection {
                reason: CallRejectionReason::OutputModeUnsupported,
                detail: RejectionDetail::new(
                    "provider does not support the requested output mode".to_owned(),
                ),
            },
        }
    }
}

/// The single origin route the agent stamps onto in-flight mail. The agent
/// serves one request per connection on its own call stack, so there is no
/// concurrent in-flight mail to disambiguate; the route is a constant.
const FORWARD_ORIGIN_ROUTE: nexus_schema::OriginRoute = nexus_schema::OriginRoute(1);

impl NexusEngine for AgentEngine {
    fn decide(
        &mut self,
        input: nexus_schema::nexus::Nexus<nexus_schema::nexus::Work>,
    ) -> nexus_schema::nexus::Nexus<nexus_schema::nexus::Action> {
        let origin_route = input.origin_route();
        let action = match input.into_root() {
            NexusWork::SignalArrived(signal_input) => self.decide_signal(signal_input),
            NexusWork::EffectCompleted(outcome) => self.decide_effect_completed(outcome),
        };
        action.with_origin_route(origin_route)
    }

    async fn run_effect(&mut self, input: CommandEffect) -> EffectCompleted {
        self.run_provider_effect(input).await
    }

    fn budget_exhausted_reply(&self, exhausted: triad_runtime::ContinuationExhausted) -> Output {
        Output::CallRejected(CallRejection {
            reason: CallRejectionReason::ProviderRejected,
            detail: RejectionDetail::new(format!(
                "nexus continuation budget exhausted after {} steps (limit {})",
                exhausted.completed_step_count(),
                exhausted.limit().count()
            )),
        })
    }
}

/// The agent holds its provider registry in the engine, not in SEMA, so its SEMA
/// engine is the honest no-op: every write and read returns `Stateless`. The
/// durable redb projection of the registry is deferred (see `schema/sema.schema`).
impl SemaEngine for AgentEngine {
    fn apply_inner(
        &mut self,
        input: sema_schema::sema::Sema<SemaWriteInput>,
    ) -> sema_schema::sema::Sema<SemaWriteOutput> {
        let origin_route = input.origin_route();
        sema_schema::sema::Sema::new(origin_route, SemaWriteOutput::Stateless(Stateless {}))
    }

    fn observe_inner(
        &self,
        input: sema_schema::sema::Sema<SemaReadInput>,
    ) -> sema_schema::sema::Sema<SemaReadOutput> {
        let origin_route = input.origin_route();
        sema_schema::sema::Sema::new(origin_route, SemaReadOutput::Stateless(Stateless {}))
    }
}
