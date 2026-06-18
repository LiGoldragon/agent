//! Fixture-provider round-trip: the daemon's call pipeline completes a prompt
//! end to end with NO live API key and NO network. This is the deliverable
//! witness that the Signal -> Nexus -> CallProvider effect -> reply path works.
//!
//! Real-network coverage is gated behind a key being present (see
//! `live_deepseek_flash_returns_valid_nota_with_gopass_key`), so CI stays
//! offline.

use agent::provider::{
    Provider, ProviderApiKey, ProviderCall, ProviderCompletion, ProviderCompletionFuture,
};
use agent::registry::{
    KeySource, KeySourceFuture, ProviderEntry, ProviderRegistry, SecretSource, SystemKeySource,
};
use agent::{AgentEngine, FixtureProvider};
#[cfg(feature = "live-provider")]
use nota_next::Document;
use signal_agent::{
    CallRejectionReason, ChatMessage, ChatTranscript, Input, MaximumOutputTokens, ModelName,
    Output, OutputMode, Prompt, PromptOptions, ProviderName, SystemText, TemperatureMilli,
};

const DEEPSEEK_PROVIDER: &str = "deepseek";
const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/v1";
const DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
const DEEPSEEK_KEY_HANDLE: &str = "DEEPSEEK_API_KEY";
#[cfg(feature = "live-provider")]
const DEEPSEEK_GOPASS_PATH: &str = "platform.deepseek.com/api-key";

/// A test key source that needs no process environment: it answers every handle
/// with a fixed literal, so a fixture call resolves without a real key.
struct LiteralKeySource;

struct StaticProvider {
    text: &'static str,
}

impl KeySource for LiteralKeySource {
    fn resolve(&self, _source: SecretSource) -> KeySourceFuture<'_> {
        Box::pin(async { Ok(ProviderApiKey::new("test-key")) })
    }
}

impl StaticProvider {
    fn new(text: &'static str) -> Self {
        Self { text }
    }
}

impl Provider for StaticProvider {
    fn complete(&self, _call: ProviderCall) -> ProviderCompletionFuture<'_> {
        let text = self.text.to_owned();
        Box::pin(async move {
            Ok(ProviderCompletion {
                text,
                stop_reason: "stop".to_owned(),
                prompt_tokens: None,
                completion_tokens: None,
            })
        })
    }
}

fn engine_with_deepseek() -> AgentEngine {
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        DEEPSEEK_PROVIDER,
        DEEPSEEK_ENDPOINT,
        DEEPSEEK_MODEL,
        SecretSource::environment(DEEPSEEK_KEY_HANDLE),
    ));
    AgentEngine::new(
        registry,
        Box::new(FixtureProvider::new()),
        Box::new(LiteralKeySource),
    )
}

fn guardian_prompt(provider: Option<&str>) -> Prompt {
    Prompt::new(
        Some(SystemText::new("You judge intent.".to_owned())),
        ChatTranscript::new(vec![ChatMessage::user(
            "Reply exactly with this NOTA expression: (Verdict accepted)",
        )]),
        PromptOptions::new(
            Some(ModelName::new(DEEPSEEK_MODEL.to_owned())),
            provider.map(|name| ProviderName::new(name.to_owned())),
            Some(TemperatureMilli::new(0)),
            Some(MaximumOutputTokens::new(64)),
            OutputMode::Nota,
            None,
            None,
        ),
    )
}

#[tokio::test]
async fn fixture_provider_completes_a_call_offline() {
    let mut engine = engine_with_deepseek();
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            DEEPSEEK_PROVIDER,
        )))))
        .await;
    match output {
        Output::Completed(completion) => {
            // The fixture returns a valid NOTA verdict; the NOTA path validates it.
            assert!(completion.completion_text.payload().contains("Verdict"));
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

#[tokio::test]
async fn nota_output_rejects_empty_document() {
    let mut engine = AgentEngine::new(
        {
            let mut registry = ProviderRegistry::new();
            registry.configure(ProviderEntry::new(
                DEEPSEEK_PROVIDER,
                DEEPSEEK_ENDPOINT,
                DEEPSEEK_MODEL,
                SecretSource::environment(DEEPSEEK_KEY_HANDLE),
            ));
            registry
        },
        Box::new(StaticProvider::new("")),
        Box::new(LiteralKeySource),
    );
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            DEEPSEEK_PROVIDER,
        )))))
        .await;
    match output {
        Output::CallRejected(rejection) => {
            assert_eq!(rejection.reason, CallRejectionReason::InvalidNotaOutput);
            assert!(
                rejection.detail.payload().contains("expected exactly one"),
                "unexpected rejection detail: {:?}",
                rejection.detail
            );
        }
        other => panic!("expected InvalidNotaOutput rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn nota_output_rejects_multiple_root_objects() {
    let mut engine = AgentEngine::new(
        {
            let mut registry = ProviderRegistry::new();
            registry.configure(ProviderEntry::new(
                DEEPSEEK_PROVIDER,
                DEEPSEEK_ENDPOINT,
                DEEPSEEK_MODEL,
                SecretSource::environment(DEEPSEEK_KEY_HANDLE),
            ));
            registry
        },
        Box::new(StaticProvider::new("Accept Accept")),
        Box::new(LiteralKeySource),
    );
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            DEEPSEEK_PROVIDER,
        )))))
        .await;
    match output {
        Output::CallRejected(rejection) => {
            assert_eq!(rejection.reason, CallRejectionReason::InvalidNotaOutput);
            assert!(
                rejection.detail.payload().contains("expected exactly one"),
                "unexpected rejection detail: {:?}",
                rejection.detail
            );
        }
        other => panic!("expected InvalidNotaOutput rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn registry_resolves_prompt_to_a_provider_call() {
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        "mimo",
        "https://api.mimo.example/v1",
        "mimo-7b",
        SecretSource::environment("MIMO_API_KEY"),
    ));
    let call: ProviderCall = registry
        .resolve(&guardian_prompt(Some("mimo")), &LiteralKeySource)
        .await
        .expect("resolve");
    assert_eq!(call.endpoint(), "https://api.mimo.example/v1");
    // The prompt named a DeepSeek model; that overrides the provider's
    // default model.
    assert_eq!(call.model(), DEEPSEEK_MODEL);
    assert!(call.is_nota());
}

#[tokio::test]
async fn system_key_source_reads_secret_file_without_trailing_newline() {
    let directory = tempfile::TempDir::new().expect("tempdir");
    let secret_path = directory.path().join("provider.key");
    std::fs::write(&secret_path, "file-secret\n").expect("write secret");
    let key = SystemKeySource
        .resolve(SecretSource::file(secret_path.display().to_string()))
        .await
        .expect("resolve file secret");
    assert_eq!(key.as_str(), "file-secret");
}

/// Real-network test, gated on a live gopass key. Skips silently when the key is
/// unavailable so CI stays offline. When `platform.deepseek.com/api-key` is
/// readable through gopass and the crate is built with `--features
/// live-provider`, this exercises the real HTTPS call.
#[cfg(feature = "live-provider")]
#[tokio::test]
async fn live_deepseek_flash_returns_valid_nota_with_gopass_key() {
    let key_available = std::process::Command::new("gopass")
        .arg("show")
        .arg("-o")
        .arg(DEEPSEEK_GOPASS_PATH)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !key_available {
        eprintln!("skipping: DeepSeek gopass key unavailable");
        return;
    }
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        DEEPSEEK_PROVIDER,
        DEEPSEEK_ENDPOINT,
        DEEPSEEK_MODEL,
        SecretSource::gopass(DEEPSEEK_GOPASS_PATH),
    ));
    let mut engine = AgentEngine::with_system_keys(
        registry,
        Box::new(agent::provider::OpenAiCompatibleProvider::new()),
    );
    let output = engine
        .handle(Input::Call(signal_agent::Call::new(guardian_prompt(Some(
            DEEPSEEK_PROVIDER,
        )))))
        .await;
    match output {
        Output::Completed(completion) => {
            let text = completion.completion_text.payload();
            Document::parse(text).expect("live DeepSeek completion must be valid NOTA");
            assert!(
                text.contains("Verdict"),
                "live completion had valid NOTA but not the requested verdict: {text}"
            );
        }
        other => panic!("live call did not complete: {other:?}"),
    }
}
