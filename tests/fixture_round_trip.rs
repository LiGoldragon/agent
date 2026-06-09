//! Fixture-provider round-trip: the daemon's call pipeline completes a prompt
//! end to end with NO live API key and NO network. This is the deliverable
//! witness that the Signal -> Nexus -> CallProvider effect -> reply path works.
//!
//! Real-network coverage is gated behind a key being present (see
//! `live_provider_completes_when_key_present`), so CI stays offline.

use agent::provider::{ProviderApiKey, ProviderCall};
use agent::registry::{KeySource, ProviderEntry, ProviderRegistry};
use agent::{AgentEngine, FixtureProvider};
use signal_agent::{
    CallRejectionReason, ChatMessage, ChatTranscript, Input, ModelName, Output, OutputMode, Prompt,
    PromptOptions, ProviderName, SystemText,
};

/// A test key source that needs no process environment: it answers every handle
/// with a fixed literal, so a fixture call resolves without a real key.
struct LiteralKeySource;

impl KeySource for LiteralKeySource {
    fn resolve(&self, _handle: &str) -> Option<ProviderApiKey> {
        Some(ProviderApiKey::new("test-key"))
    }
}

fn engine_with_deepseek() -> AgentEngine {
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        "deepseek",
        "https://api.deepseek.com/v1",
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    ));
    AgentEngine::new(
        registry,
        Box::new(FixtureProvider::new()),
        Box::new(LiteralKeySource),
    )
}

fn guardian_prompt(provider: Option<&str>) -> Prompt {
    Prompt {
        system: Some(SystemText::new("You judge intent.".to_owned())),
        transcript: ChatTranscript::new(vec![ChatMessage::user("Is this a durable decision?")]),
        options: PromptOptions {
            model: Some(ModelName::new("deepseek-chat".to_owned())),
            provider: provider.map(|name| ProviderName::new(name.to_owned())),
            temperature_milli: None,
            maximum_output_tokens: None,
            output_mode: OutputMode::JsonObject,
        },
    }
}

#[tokio::test]
async fn fixture_provider_completes_a_call_offline() {
    let mut engine = engine_with_deepseek();
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            "deepseek",
        )))))
        .await;
    match output {
        Output::Completed(completion) => {
            // The fixture echoes the last user turn into a JSON verdict.
            assert!(completion.text.payload().contains("verdict"));
            assert_eq!(completion.stop_reason.payload(), "stop");
        }
        other => panic!("expected a completion, got {other:?}"),
    }
}

#[tokio::test]
async fn call_with_no_configured_provider_is_rejected() {
    let mut engine = AgentEngine::new(
        ProviderRegistry::new(),
        Box::new(FixtureProvider::new()),
        Box::new(LiteralKeySource),
    );
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(None))))
        .await;
    match output {
        Output::CallRejected(rejection) => {
            assert_eq!(rejection.reason, CallRejectionReason::NoProviderConfigured);
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn default_provider_is_used_when_prompt_names_none() {
    let mut engine = engine_with_deepseek();
    // No provider named in the prompt; the registry default (deepseek, the first
    // configured) resolves the call.
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(None))))
        .await;
    assert!(matches!(output, Output::Completed(_)));
}

#[test]
fn registry_resolves_prompt_to_a_provider_call() {
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        "mimo",
        "https://api.mimo.example/v1",
        "mimo-7b",
        "MIMO_API_KEY",
    ));
    let call: ProviderCall = registry
        .resolve(&guardian_prompt(Some("mimo")), &LiteralKeySource)
        .expect("resolve");
    assert_eq!(call.endpoint(), "https://api.mimo.example/v1");
    // The prompt named deepseek-chat as the model; that overrides the provider's
    // default model.
    assert_eq!(call.model(), "deepseek-chat");
    assert!(call.is_json_object());
}

/// Real-network test, gated on a live key. Skips silently when no key is set so
/// CI stays offline. When `DEEPSEEK_API_KEY` is present and the crate is built
/// with `--features live-provider`, this exercises the real HTTPS call.
#[cfg(feature = "live-provider")]
#[tokio::test]
async fn live_provider_completes_when_key_present() {
    let Ok(_key) = std::env::var("DEEPSEEK_API_KEY") else {
        eprintln!("skipping: DEEPSEEK_API_KEY not set");
        return;
    };
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        "deepseek",
        "https://api.deepseek.com/v1",
        "deepseek-chat",
        "DEEPSEEK_API_KEY",
    ));
    let mut engine = AgentEngine::with_environment_keys(
        registry,
        Box::new(agent::provider::OpenAiCompatibleProvider::new()),
    );
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            "deepseek",
        )))))
        .await;
    assert!(
        matches!(output, Output::Completed(_)),
        "live call did not complete: {output:?}"
    );
}
