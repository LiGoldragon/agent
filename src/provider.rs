//! The provider-call plane: how the daemon makes one OpenAI-compatible
//! `/chat/completions` request.
//!
//! A `Provider` is the async effect behind the Nexus `CallProvider` command. The
//! daemon resolves a `ProviderCall` (endpoint + model + resolved API key + the
//! chat messages) from the registry, then asks the provider to `complete` it.
//! Two implementations:
//!
//! - `FixtureProvider` — a deterministic mock that needs no network and no API
//!   key, so the crate builds and the round-trip test runs offline.
//! - `OpenAiCompatibleProvider` — the reqwest-backed real call (feature
//!   `live-provider`), gated so the default build carries no network stack.
//!
//! Adding DeepSeek, MiMo, Kimi, GLM, or MiniMax is configuration: they all speak
//! this one OpenAI-compatible shape, so the same `OpenAiCompatibleProvider`
//! serves every configured provider — only the endpoint, model, and key differ.

use signal_agent::{ChatRole, OutputMode, ReasoningEffort, ThinkingMode};

/// One fully-resolved provider call: the registry has already turned a
/// `ProviderName` into an endpoint, a model, and the resolved secret API key.
/// The secret lives only inside this value while the call is in flight; it is
/// never stored, logged, or sent anywhere but the provider's TLS endpoint.
#[derive(Debug, Clone)]
pub struct ProviderCall {
    endpoint: String,
    model: String,
    api_key: ProviderApiKey,
    system: Option<String>,
    messages: Vec<ProviderMessage>,
    output_mode: OutputMode,
    temperature_milli: Option<u64>,
    maximum_output_tokens: Option<u64>,
    reasoning_effort: Option<ReasoningEffort>,
    thinking_mode: Option<ThinkingMode>,
}

/// The instruction the daemon folds into the system message for `OutputMode::Nota`:
/// the model must emit exactly one valid NOTA expression. NOTA has no provider-level
/// constrained-decode mode (unlike JSON's `response_format`), so the daemon instructs,
/// then validates the completion parses as NOTA and retries once before rejecting.
const NOTA_OUTPUT_INSTRUCTION: &str = "Respond with exactly one valid NOTA s-expression and nothing else. NOTA uses bracket forms: parenthesised records like (Head field ...), bracketed strings like [free text], and bare camelCase or kebab-case tokens. Do not use quotation marks, markdown fences, or any prose outside the expression.";

impl ProviderCall {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: ProviderApiKey,
        system: Option<String>,
        messages: Vec<ProviderMessage>,
        output_mode: OutputMode,
        temperature_milli: Option<u64>,
        maximum_output_tokens: Option<u64>,
        reasoning_effort: Option<ReasoningEffort>,
        thinking_mode: Option<ThinkingMode>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key,
            system,
            messages,
            output_mode,
            temperature_milli,
            maximum_output_tokens,
            reasoning_effort,
            thinking_mode,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn api_key(&self) -> &ProviderApiKey {
        &self.api_key
    }

    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    pub fn messages(&self) -> &[ProviderMessage] {
        &self.messages
    }

    pub fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    pub fn is_nota(&self) -> bool {
        matches!(self.output_mode, OutputMode::Nota)
    }

    pub fn temperature(&self) -> Option<f64> {
        self.temperature_milli.map(|milli| milli as f64 / 1000.0)
    }

    pub fn maximum_output_tokens(&self) -> Option<u64> {
        self.maximum_output_tokens
    }

    /// The OpenAI-compatible `reasoning_effort` string DeepSeek expects
    /// (`low`/`medium`/`high`), derived from the typed `ReasoningEffort`.
    pub fn reasoning_effort(&self) -> Option<&'static str> {
        match self.reasoning_effort.as_ref()? {
            ReasoningEffort::Low => Some("low"),
            ReasoningEffort::Medium => Some("medium"),
            ReasoningEffort::High => Some("high"),
        }
    }

    /// The DeepSeek thinking toggle value (`enabled`/`disabled`) for the
    /// top-level `thinking` object, derived from the typed `ThinkingMode`.
    pub fn thinking_directive(&self) -> Option<&'static str> {
        match self.thinking_mode.as_ref()? {
            ThinkingMode::Enabled => Some("enabled"),
            ThinkingMode::Disabled => Some("disabled"),
        }
    }

    /// A copy of this call with the NOTA-output instruction folded into the system
    /// message — used for `OutputMode::Nota` so the model emits NOTA directly.
    pub fn with_nota_instruction(&self) -> Self {
        let system = match &self.system {
            Some(existing) => format!("{existing}\n\n{NOTA_OUTPUT_INSTRUCTION}"),
            None => NOTA_OUTPUT_INSTRUCTION.to_owned(),
        };
        Self {
            system: Some(system),
            ..self.clone()
        }
    }

    /// A copy of this call extended with the model's invalid previous answer and a
    /// correction turn — the single retry the daemon makes when NOTA validation fails.
    pub fn with_nota_correction(&self, previous: &str, parse_error: &str) -> Self {
        let mut messages = self.messages.clone();
        messages.push(ProviderMessage::new(
            ChatRole::Assistant,
            previous.to_owned(),
        ));
        messages.push(ProviderMessage::new(
            ChatRole::User,
            format!(
                "That response was not valid NOTA ({parse_error}). Reply with exactly one valid NOTA expression and nothing else."
            ),
        ));
        Self {
            messages,
            ..self.clone()
        }
    }
}

/// A resolved secret API key. Held only for the duration of one call; its
/// `Debug` is redacted so a stray log never leaks the value.
#[derive(Clone)]
pub struct ProviderApiKey(String);

impl ProviderApiKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn from_secret_output(value: impl Into<String>) -> Self {
        Self(value.into().trim_end_matches(['\r', '\n']).to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for ProviderApiKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProviderApiKey(<redacted>)")
    }
}

/// One chat turn handed to a provider, projected from the contract `ChatRole`.
#[derive(Debug, Clone)]
pub struct ProviderMessage {
    role: ChatRole,
    content: String,
}

impl ProviderMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn role_name(&self) -> &'static str {
        match self.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

/// The successful provider result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompletion {
    pub text: String,
    pub stop_reason: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

/// A typed provider-call failure. The daemon maps each variant to a
/// `CallRejectionReason` on the wire.
#[derive(Debug, Clone)]
pub enum ProviderFailure {
    Unreachable(String),
    ProviderRejected(String),
    OutputModeUnsupported,
}

/// The future a `Provider::complete` returns. Boxed so `Provider` is
/// dyn-compatible — the engine holds `Box<dyn Provider>` to share one engine
/// type across the fixture and the live reqwest backend.
pub type ProviderCompletionFuture<'provider> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<ProviderCompletion, ProviderFailure>>
            + Send
            + 'provider,
    >,
>;

/// The provider plane. One async method: make the call. Implementors own the
/// transport (reqwest, or the fixture's in-memory canned answer). The boxed
/// future keeps the trait dyn-compatible.
pub trait Provider: Send + Sync {
    fn complete(&self, call: ProviderCall) -> ProviderCompletionFuture<'_>;
}

/// A deterministic offline provider. It performs no network IO and ignores the
/// API key, so the daemon builds and its round-trip test runs with no live key
/// and no network. It echoes a fixed verdict shaped by the requested output
/// mode, which is exactly what a fixture round-trip needs to witness.
#[derive(Debug, Clone, Default)]
pub struct FixtureProvider;

impl FixtureProvider {
    pub fn new() -> Self {
        Self
    }
}

impl FixtureProvider {
    fn fixture_completion(call: &ProviderCall) -> ProviderCompletion {
        let last_user = call
            .messages()
            .iter()
            .rev()
            .find(|message| message.role_name() == "user")
            .map(|message| message.content())
            .unwrap_or("");
        let text = match call.output_mode() {
            OutputMode::Nota => "(Verdict accepted)".to_owned(),
            OutputMode::FreeText => format!("fixture completion for: {last_user}"),
        };
        ProviderCompletion {
            text,
            stop_reason: "stop".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
        }
    }
}

impl Provider for FixtureProvider {
    fn complete(&self, call: ProviderCall) -> ProviderCompletionFuture<'_> {
        Box::pin(async move { Ok(Self::fixture_completion(&call)) })
    }
}

#[cfg(feature = "live-provider")]
pub use live::OpenAiCompatibleProvider;

#[cfg(feature = "live-provider")]
mod live {
    use super::{Provider, ProviderCall, ProviderCompletion, ProviderFailure};
    use serde::{Deserialize, Serialize};

    /// The reqwest-backed OpenAI-compatible provider. One client serves every
    /// configured provider — DeepSeek, MiMo, Kimi, GLM, MiniMax — because they
    /// all expose the same `/chat/completions` shape; only the endpoint, model,
    /// and key (carried per-call in `ProviderCall`) differ.
    #[derive(Debug, Clone)]
    pub struct OpenAiCompatibleProvider {
        client: reqwest::Client,
    }

    impl OpenAiCompatibleProvider {
        pub fn new() -> Self {
            Self {
                client: reqwest::Client::new(),
            }
        }
    }

    impl Default for OpenAiCompatibleProvider {
        fn default() -> Self {
            Self::new()
        }
    }

    impl OpenAiCompatibleProvider {
        async fn complete_call(
            &self,
            call: ProviderCall,
        ) -> Result<ProviderCompletion, ProviderFailure> {
            let request = ChatCompletionRequest::from_call(&call);
            let url = format!("{}/chat/completions", call.endpoint().trim_end_matches('/'));
            let response = self
                .client
                .post(url)
                .bearer_auth(call.api_key().as_str())
                .json(&request)
                .send()
                .await
                .map_err(|error| ProviderFailure::Unreachable(error.to_string()))?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderFailure::ProviderRejected(format!(
                    "status {status}: {body}"
                )));
            }
            let completion: ChatCompletionResponse = response
                .json()
                .await
                .map_err(|error| ProviderFailure::ProviderRejected(error.to_string()))?;
            completion.into_provider_completion()
        }
    }

    impl Provider for OpenAiCompatibleProvider {
        fn complete(&self, call: ProviderCall) -> super::ProviderCompletionFuture<'_> {
            Box::pin(self.complete_call(call))
        }
    }

    #[derive(Debug, Serialize)]
    struct ChatCompletionRequest {
        model: String,
        messages: Vec<ChatCompletionMessage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_effort: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<ThinkingDirective>,
    }

    /// DeepSeek's top-level `thinking` toggle object: `{"type": "enabled"}`.
    #[derive(Debug, Serialize)]
    struct ThinkingDirective {
        #[serde(rename = "type")]
        kind: &'static str,
    }

    impl ChatCompletionRequest {
        fn from_call(call: &ProviderCall) -> Self {
            let mut messages = Vec::new();
            if let Some(system) = call.system() {
                messages.push(ChatCompletionMessage {
                    role: "system".to_owned(),
                    content: system.to_owned(),
                });
            }
            for message in call.messages() {
                messages.push(ChatCompletionMessage {
                    role: message.role_name().to_owned(),
                    content: message.content().to_owned(),
                });
            }
            Self {
                model: call.model().to_owned(),
                messages,
                temperature: call.temperature(),
                max_tokens: call.maximum_output_tokens(),
                reasoning_effort: call.reasoning_effort(),
                thinking: call
                    .thinking_directive()
                    .map(|kind| ThinkingDirective { kind }),
            }
        }
    }

    #[derive(Debug, Serialize)]
    struct ChatCompletionMessage {
        role: String,
        content: String,
    }

    #[derive(Debug, Deserialize)]
    struct ChatCompletionResponse {
        choices: Vec<ChatCompletionChoice>,
        usage: Option<ChatCompletionUsage>,
    }

    impl ChatCompletionResponse {
        fn into_provider_completion(self) -> Result<ProviderCompletion, ProviderFailure> {
            let usage = self.usage;
            let choice = self.choices.into_iter().next().ok_or_else(|| {
                ProviderFailure::ProviderRejected("response carried no choices".to_owned())
            })?;
            Ok(ProviderCompletion {
                text: choice.message.content,
                stop_reason: choice.finish_reason.unwrap_or_else(|| "stop".to_owned()),
                prompt_tokens: usage.as_ref().and_then(|usage| usage.prompt_tokens),
                completion_tokens: usage.and_then(|usage| usage.completion_tokens),
            })
        }
    }

    #[derive(Debug, Deserialize)]
    struct ChatCompletionChoice {
        message: ChatCompletionResponseMessage,
        finish_reason: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct ChatCompletionResponseMessage {
        content: String,
    }

    #[derive(Debug, Deserialize)]
    struct ChatCompletionUsage {
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
    }
}
