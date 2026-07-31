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
//! `CommandEffect(CallProvider(Prompt))`; `execute` awaits the provider effect,
//! which resolves the registry and asks the `Provider` to complete the call —
//! the only place the daemon touches the network, and it does so off the engine
//! mailbox through the async effect seam, never blocking a handler.

use dotos::Document;
use signal_agent::{
    CallRejection, CallRejectionReason, Completion, CompletionText, CompletionTokenCount,
    OperationKind, Output, OutputMode, PromptTokenCount, RejectionDetail, RequestUnimplemented,
    StopReasonText, TokenUsage, UnimplementedReason,
};

use crate::provider::{Provider, ProviderCall, ProviderCompletion, ProviderFailure};
use crate::registry::{KeySource, ProviderEntry, ProviderRegistry, ResolveError, SystemKeySource};
use crate::schema::nexus::{
    self as nexus_schema, NexusAction, NexusEngine, NexusWork, ProviderCallCommand, ProviderOutcome,
};
use crate::schema::sema::{
    self as sema_schema, SemaEngine, SemaReadInput, SemaReadOutput, SemaWriteInput,
    SemaWriteOutput, Stateless,
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
    pub fn with_system_keys(registry: ProviderRegistry, provider: Box<dyn Provider>) -> Self {
        Self::new(registry, provider, Box::new(SystemKeySource))
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
        let prompt = match input {
            signal_agent::Input::Call(call) => call.into_payload(),
            signal_agent::Input::StreamCall(_) => {
                return Self::unimplemented_reply(OperationKind::StreamCall);
            }
            signal_agent::Input::CancelStream(_) => {
                return Self::unimplemented_reply(OperationKind::CancelStream);
            }
        };
        let work =
            NexusWork::signal_arrived(prompt).with_origin_route(Self::forward_origin_route());
        let action = self.execute(work).await.into_root();
        match action {
            NexusAction::ReplyToSignal(outcome) => Self::output_from_outcome(outcome),
            other => Output::CallRejected(CallRejection {
                call_rejection_reason: CallRejectionReason::ProviderRejected,
                rejection_detail: RejectionDetail::new(format!(
                    "nexus runner returned non-reply action: {other:?}"
                )),
            }),
        }
    }

    /// The decision for an arrived ordinary call prompt.
    fn decide_signal(&self, prompt: signal_agent::Prompt) -> NexusAction {
        NexusAction::command_effect(ProviderCallCommand::call_provider(prompt))
    }

    /// Turn a completed provider call into the local Nexus reply payload.
    fn decide_effect_completed(&self, outcome: ProviderOutcome) -> NexusAction {
        NexusAction::reply_to_signal(outcome)
    }

    /// Convert the local provider outcome back into the public signal contract.
    fn output_from_outcome(outcome: ProviderOutcome) -> Output {
        match outcome {
            ProviderOutcome::Completed(completion) => Output::Completed(completion),
            ProviderOutcome::Rejected(rejection) => Output::CallRejected(rejection),
        }
    }

    fn unimplemented_reply(operation_kind: OperationKind) -> Output {
        Output::RequestUnimplemented(RequestUnimplemented {
            operation_kind,
            unimplemented_reason: UnimplementedReason::NotInPrototypeScope,
        })
    }

    /// Run the one effect the agent declares: call the configured provider. This
    /// resolves the prompt against the registry (provider, model, secret source),
    /// makes the OpenAI-compatible call through the `Provider`, and lifts the
    /// result into a typed `ProviderOutcome`.
    async fn run_provider_effect(&self, command: ProviderCallCommand) -> ProviderOutcome {
        let ProviderCallCommand::CallProvider(prompt) = command;
        match self.registry.resolve(&prompt, self.keys.as_ref()).await {
            Ok(call) => self.complete_call(call).await,
            Err(error) => ProviderOutcome::rejected(Self::resolve_rejection(error)),
        }
    }

    async fn complete_call(&self, call: ProviderCall) -> ProviderOutcome {
        match call.output_mode() {
            OutputMode::FreeText => self.complete_once(call).await,
            OutputMode::Dotos => self.complete_dotos(call).await,
        }
    }

    /// One provider call, no output validation — the `FreeText` path.
    async fn complete_once(&self, call: ProviderCall) -> ProviderOutcome {
        match self.provider.complete(call).await {
            Ok(completion) => ProviderOutcome::completed(Self::completion(completion)),
            Err(failure) => ProviderOutcome::rejected(Self::failure_rejection(failure)),
        }
    }

    /// The DOTOS path: the model emits DOTOS directly. Inject the DOTOS instruction,
    /// validate the completion parses as DOTOS, and retry once with the parse error
    /// before rejecting with `InvalidDotosOutput`. DOTOS has no provider-level
    /// constrained-decode mode, so validate-and-retry is the reliability mechanism.
    /// This runs inside the async `run_effect` seam, off the engine mailbox.
    async fn complete_dotos(&self, call: ProviderCall) -> ProviderOutcome {
        let mut attempt = call.with_dotos_instruction();
        let mut last_error = String::new();
        for _ in 0..DOTOS_OUTPUT_ATTEMPTS {
            match self.provider.complete(attempt.clone()).await {
                Ok(completion) => match Self::validate_dotos_completion(completion.text.as_str()) {
                    Ok(_) => return ProviderOutcome::completed(Self::completion(completion)),
                    Err(error) => {
                        last_error = error;
                        attempt = attempt.with_dotos_correction(&completion.text, &last_error);
                    }
                },
                Err(failure) => return ProviderOutcome::rejected(Self::failure_rejection(failure)),
            }
        }
        ProviderOutcome::rejected(Self::invalid_dotos_rejection(&last_error))
    }

    fn validate_dotos_completion(text: &str) -> Result<(), String> {
        let document = Document::parse(text).map_err(|error| error.to_string())?;
        if document.holds_root_objects() == 1 {
            Ok(())
        } else {
            Err(format!(
                "expected exactly one DOTOS root object, found {}",
                document.holds_root_objects()
            ))
        }
    }

    fn completion(completion: ProviderCompletion) -> Completion {
        Completion {
            completion_text: CompletionText::new(completion.text),
            stop_reason_text: StopReasonText::new(completion.stop_reason),
            token_usage: TokenUsage::new(
                completion.prompt_tokens.map(PromptTokenCount::new),
                completion.completion_tokens.map(CompletionTokenCount::new),
            ),
        }
    }

    fn resolve_rejection(error: ResolveError) -> CallRejection {
        match error {
            ResolveError::NoProviderConfigured => CallRejection {
                call_rejection_reason: CallRejectionReason::NoProviderConfigured,
                rejection_detail: RejectionDetail::new(
                    "no provider configured and prompt named none".to_owned(),
                ),
            },
            ResolveError::ProviderUnknown(name) => CallRejection {
                call_rejection_reason: CallRejectionReason::NoProviderConfigured,
                rejection_detail: RejectionDetail::new(format!("provider not in registry: {name}")),
            },
            ResolveError::SecretUnavailable(error) => CallRejection {
                call_rejection_reason: CallRejectionReason::DaemonUnconfigured,
                rejection_detail: RejectionDetail::new(format!("secret unavailable: {error}")),
            },
        }
    }

    fn failure_rejection(failure: ProviderFailure) -> CallRejection {
        match failure {
            ProviderFailure::Unreachable(detail) => CallRejection {
                call_rejection_reason: CallRejectionReason::ProviderUnreachable,
                rejection_detail: RejectionDetail::new(detail),
            },
            ProviderFailure::ProviderRejected(detail) => CallRejection {
                call_rejection_reason: CallRejectionReason::ProviderRejected,
                rejection_detail: RejectionDetail::new(detail),
            },
            ProviderFailure::OutputModeUnsupported => CallRejection {
                call_rejection_reason: CallRejectionReason::OutputModeUnsupported,
                rejection_detail: RejectionDetail::new(
                    "provider does not support the requested output mode".to_owned(),
                ),
            },
        }
    }

    fn invalid_dotos_rejection(last_error: &str) -> CallRejection {
        CallRejection {
            call_rejection_reason: CallRejectionReason::InvalidDotosOutput,
            rejection_detail: RejectionDetail::new(format!(
                "model did not produce valid DOTOS after {DOTOS_OUTPUT_ATTEMPTS} attempts: {last_error}"
            )),
        }
    }

    fn budget_exhausted_outcome(
        &self,
        exhausted: triad_runtime::ContinuationExhausted,
    ) -> ProviderOutcome {
        ProviderOutcome::rejected(CallRejection {
            call_rejection_reason: CallRejectionReason::ProviderRejected,
            rejection_detail: RejectionDetail::new(format!(
                "nexus continuation budget exhausted after {} steps (limit {})",
                exhausted.completed_step_count(),
                exhausted.limit().count()
            )),
        })
    }

    /// The single origin route the agent stamps onto in-flight mail. The agent
    /// serves one request per connection on its own call stack, so there is no
    /// concurrent in-flight mail to disambiguate; the route is a constant.
    fn forward_origin_route() -> nexus_schema::OriginRoute {
        nexus_schema::OriginRoute::new(1)
    }
}

/// How many times the DOTOS path asks the model for valid DOTOS: the first attempt
/// plus one correction retry. DOTOS has no provider-level constrained-decode mode,
/// so a bounded validate-and-retry is the reliability mechanism.
const DOTOS_OUTPUT_ATTEMPTS: usize = 2;

impl NexusEngine for AgentEngine {
    fn run_effect(
        &mut self,
        input: ProviderCallCommand,
    ) -> impl std::future::Future<Output = ProviderOutcome> + Send + '_ {
        self.run_provider_effect(input)
    }

    fn budget_exhausted_reply(
        &self,
        exhausted: triad_runtime::ContinuationExhausted,
    ) -> ProviderOutcome {
        self.budget_exhausted_outcome(exhausted)
    }

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

    async fn execute(
        &mut self,
        input: nexus_schema::nexus::Nexus<nexus_schema::nexus::Work>,
    ) -> nexus_schema::nexus::Nexus<nexus_schema::nexus::Action> {
        let origin_route = input.origin_route();
        let mut work = input;
        let mut budget = triad_runtime::ContinuationLimit::default().budget();
        loop {
            if let Err(exhausted) = budget.spend_next_step() {
                return NexusAction::reply_to_signal(self.budget_exhausted_outcome(exhausted))
                    .with_origin_route(origin_route);
            }
            self.trace_nexus_entered();
            let action = self.decide(work).into_root();
            self.trace_nexus_decided();
            match action {
                NexusAction::ReplyToSignal(_) => {
                    return action.with_origin_route(origin_route);
                }
                NexusAction::CommandEffect(effect) => {
                    if let Err(exhausted) = budget.spend_next_step() {
                        return NexusAction::reply_to_signal(
                            self.budget_exhausted_outcome(exhausted),
                        )
                        .with_origin_route(origin_route);
                    }
                    let outcome = self.run_provider_effect(effect).await;
                    work = NexusWork::effect_completed(outcome).with_origin_route(origin_route);
                }
                NexusAction::Continue(continuation) => {
                    work = continuation.with_origin_route(origin_route);
                }
            }
        }
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
        sema_schema::sema::Sema::new(origin_route, SemaWriteOutput::NoDurableState(Stateless {}))
    }

    fn observe_inner(
        &self,
        input: sema_schema::sema::Sema<SemaReadInput>,
    ) -> sema_schema::sema::Sema<SemaReadOutput> {
        let origin_route = input.origin_route();
        sema_schema::sema::Sema::new(origin_route, SemaReadOutput::NoDurableState(Stateless {}))
    }
}
