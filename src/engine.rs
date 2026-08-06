//! The Agent engine: contract input, one provider effect, contract output.

use dotos::Document;
use signal_agent::{
    z2VMMe, z2VNAv, z2VQ3h, z2VQBL, z2VRg8, z2VSHd, z2VSnT, z2VTYs, z2VUN4, z2VXAa, z2VZf9, z2Vb6p,
    z2VbCg, z2Ve6M,
};

use crate::provider::{Provider, ProviderCall, ProviderCompletion, ProviderFailure};
use crate::registry::{KeySource, ProviderEntry, ProviderRegistry, ResolveError, SystemKeySource};

/// Provider policy and the one implementation of the provider-call effect.
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

    pub fn with_system_keys(registry: ProviderRegistry, provider: Box<dyn Provider>) -> Self {
        Self::new(registry, provider, Box::new(SystemKeySource))
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut ProviderRegistry {
        &mut self.registry
    }

    pub fn configure_provider(&mut self, entry: ProviderEntry) {
        self.registry.configure(entry);
    }

    pub async fn handle(&mut self, input: z2VNAv) -> z2Ve6M {
        let prompt = match input {
            z2VNAv::z2VRrf(call) => call.into_payload(),
            z2VNAv::z2VTYr(_) => {
                return Self::unimplemented_reply(z2VZf9::z2Vdwh);
            }
            z2VNAv::z2VPoN(_) => {
                return Self::unimplemented_reply(z2VZf9::z2VRkZ);
            }
        };
        match self.run_provider_effect(prompt).await {
            Ok(completion) => z2Ve6M::z2VRXc(completion),
            Err(rejection) => z2Ve6M::z2VR63(rejection),
        }
    }

    fn unimplemented_reply(operation: z2VZf9) -> z2Ve6M {
        z2Ve6M::z2Vc1N(z2VQBL {
            field_0: operation,
            field_1: z2VSnT::z2VVPv,
        })
    }

    async fn run_provider_effect(&self, prompt: z2VMMe) -> Result<z2VXAa, z2VQ3h> {
        match self.registry.resolve(&prompt, self.keys.as_ref()).await {
            Ok(call) => self.complete_call(call).await,
            Err(error) => Err(Self::resolve_rejection(error)),
        }
    }

    async fn complete_call(&self, call: ProviderCall) -> Result<z2VXAa, z2VQ3h> {
        match call.output_mode() {
            z2VSHd::z2Va7b => self.complete_once(call).await,
            z2VSHd::z2VPna => self.complete_dotos(call).await,
        }
    }

    async fn complete_once(&self, call: ProviderCall) -> Result<z2VXAa, z2VQ3h> {
        self.provider
            .complete(call)
            .await
            .map(Self::completion)
            .map_err(Self::failure_rejection)
    }

    async fn complete_dotos(&self, call: ProviderCall) -> Result<z2VXAa, z2VQ3h> {
        let mut attempt = call.with_dotos_instruction();
        let mut last_error = String::new();
        for _ in 0..DOTOS_OUTPUT_ATTEMPTS {
            match self.provider.complete(attempt.clone()).await {
                Ok(completion) => match Self::validate_dotos_completion(completion.text.as_str()) {
                    Ok(()) => return Ok(Self::completion(completion)),
                    Err(error) => {
                        last_error = error;
                        attempt = attempt.with_dotos_correction(&completion.text, &last_error);
                    }
                },
                Err(failure) => return Err(Self::failure_rejection(failure)),
            }
        }
        Err(Self::invalid_dotos_rejection(&last_error))
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

    fn completion(completion: ProviderCompletion) -> z2VXAa {
        z2VXAa {
            field_0: z2VTYs::new(completion.text),
            field_1: z2VbCg::new(completion.stop_reason),
            field_2: z2Vb6p {
                field_0: completion.prompt_tokens.map(signal_agent::z2VecB::new),
                field_1: completion.completion_tokens.map(signal_agent::z2VTNt::new),
            },
        }
    }

    fn rejection(reason: z2VUN4, detail: String) -> z2VQ3h {
        z2VQ3h {
            field_0: reason,
            field_1: z2VRg8::new(detail),
        }
    }

    fn resolve_rejection(error: ResolveError) -> z2VQ3h {
        match error {
            ResolveError::NoProviderConfigured => Self::rejection(
                z2VUN4::z2VS9d,
                "no provider configured and prompt named none".to_owned(),
            ),
            ResolveError::ProviderUnknown(name) => {
                Self::rejection(z2VUN4::z2VS9d, format!("provider not in registry: {name}"))
            }
            ResolveError::SecretUnavailable(error) => {
                Self::rejection(z2VUN4::z2VZ8M, format!("secret unavailable: {error}"))
            }
        }
    }

    fn failure_rejection(failure: ProviderFailure) -> z2VQ3h {
        match failure {
            ProviderFailure::Unreachable(detail) => Self::rejection(z2VUN4::z2VdLV, detail),
            ProviderFailure::ProviderRejected(detail) => Self::rejection(z2VUN4::z2VYdH, detail),
            ProviderFailure::OutputModeUnsupported => Self::rejection(
                z2VUN4::z2VTxt,
                "provider does not support the requested output mode".to_owned(),
            ),
        }
    }

    fn invalid_dotos_rejection(last_error: &str) -> z2VQ3h {
        Self::rejection(
            z2VUN4::z2VQdg,
            format!(
                "model did not produce valid DOTOS after {DOTOS_OUTPUT_ATTEMPTS} attempts: {last_error}"
            ),
        )
    }
}

const DOTOS_OUTPUT_ATTEMPTS: usize = 2;
