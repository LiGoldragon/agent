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
use nota::Document;
#[cfg(feature = "live-provider")]
use serde_json::Value;
use signal_agent::{
    CallRejectionReason, ChatMessage, ChatTranscript, Input, MaximumOutputTokens, ModelName,
    Output, OutputMode, Prompt, PromptOptions, ProviderName, SystemText, TemperatureMilli,
};
#[cfg(feature = "live-provider")]
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

const DEEPSEEK_PROVIDER: &str = "deepseek";
const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/v1";
const DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
const DEEPSEEK_KEY_HANDLE: &str = "DEEPSEEK_API_KEY";
#[cfg(feature = "live-provider")]
const LOCAL_OPENAI_PROVIDER: &str = "local-openai";
#[cfg(feature = "live-provider")]
const LOCAL_OPENAI_MODEL: &str = "gpt-5.5";
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

#[cfg(feature = "live-provider")]
fn provider_prompt(provider: &str, model: &str) -> Prompt {
    Prompt::new(
        Some(SystemText::new("You classify one record.".to_owned())),
        ChatTranscript::new(vec![ChatMessage::user("Return (Verdict accepted)")]),
        PromptOptions::new(
            Some(ModelName::new(model.to_owned())),
            Some(ProviderName::new(provider.to_owned())),
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
            // The fixture returns a valid generic NOTA expression; domain-specific
            // response contracts belong to the caller prompt, not the provider fixture.
            assert_eq!(
                completion.completion_text.payload(),
                "(FixtureCompletion ok)"
            );
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

#[cfg(feature = "live-provider")]
struct CapturedHttpRequest {
    request_line: String,
    authorization: Option<String>,
    body: Value,
}

#[cfg(feature = "live-provider")]
struct CapturingOpenAiServer {
    endpoint: String,
    thread: thread::JoinHandle<CapturedHttpRequest>,
}

#[cfg(feature = "live-provider")]
impl CapturingOpenAiServer {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local capture server");
        let endpoint = format!("http://{}/v1", listener.local_addr().expect("local addr"));
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 4096];
            let header_end = loop {
                let count = stream.read(&mut buffer).expect("read request");
                assert!(count > 0, "provider closed before headers");
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length: ")
                        .or_else(|| line.strip_prefix("Content-Length: "))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("content-length header");
            while bytes.len() < header_end + content_length {
                let count = stream.read(&mut buffer).expect("read body");
                assert!(count > 0, "provider closed before body");
                bytes.extend_from_slice(&buffer[..count]);
            }
            let body_slice = &bytes[header_end..header_end + content_length];
            let body: Value = serde_json::from_slice(body_slice).expect("request json");
            let authorization = headers.lines().find_map(|line| {
                line.strip_prefix("authorization: ")
                    .or_else(|| line.strip_prefix("Authorization: "))
                    .map(ToOwned::to_owned)
            });
            let request_line = headers.lines().next().unwrap_or_default().to_owned();
            let response_body = "{\"choices\":[{\"message\":{\"content\":\"(Verdict accepted)\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            CapturedHttpRequest {
                request_line,
                authorization,
                body,
            }
        });
        Self { endpoint, thread }
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn join(self) -> CapturedHttpRequest {
        self.thread.join().expect("capture thread joins")
    }
}

#[cfg(feature = "live-provider")]
#[tokio::test]
async fn local_openai_compatible_provider_omits_authorization_for_no_secret() {
    let server = CapturingOpenAiServer::spawn();
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        LOCAL_OPENAI_PROVIDER,
        server.endpoint(),
        LOCAL_OPENAI_MODEL,
        SecretSource::no_secret(),
    ));
    let call = registry
        .resolve(
            &provider_prompt(LOCAL_OPENAI_PROVIDER, LOCAL_OPENAI_MODEL),
            &LiteralKeySource,
        )
        .await
        .expect("resolve local provider");
    let completion = agent::provider::OpenAiCompatibleProvider::new()
        .complete(call)
        .await
        .expect("provider completion");
    assert_eq!(completion.text, "(Verdict accepted)");

    let captured = server.join();
    assert_eq!(captured.request_line, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(captured.authorization, None);
    assert_eq!(captured.body["model"], LOCAL_OPENAI_MODEL);
    assert!(captured.body.get("tools").is_none());
    assert!(captured.body.get("tool_choice").is_none());
    assert_eq!(captured.body["messages"][0]["role"], "system");
    assert_eq!(captured.body["messages"][1]["role"], "user");
}

#[cfg(feature = "live-provider")]
#[tokio::test]
async fn local_openai_compatible_provider_sends_configured_bearer_header() {
    let server = CapturingOpenAiServer::spawn();
    let mut registry = ProviderRegistry::new();
    registry.configure(ProviderEntry::new(
        LOCAL_OPENAI_PROVIDER,
        server.endpoint(),
        LOCAL_OPENAI_MODEL,
        SecretSource::environment("LOCAL_OPENAI_API_KEY"),
    ));
    let call = registry
        .resolve(
            &provider_prompt(LOCAL_OPENAI_PROVIDER, LOCAL_OPENAI_MODEL),
            &LiteralKeySource,
        )
        .await
        .expect("resolve local provider");
    let completion = agent::provider::OpenAiCompatibleProvider::new()
        .complete(call)
        .await
        .expect("provider completion");
    assert_eq!(completion.text, "(Verdict accepted)");

    let captured = server.join();
    assert_eq!(captured.request_line, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(captured.authorization.as_deref(), Some("Bearer test-key"));
    assert_eq!(captured.body["model"], LOCAL_OPENAI_MODEL);
    assert!(captured.body.get("tools").is_none());
    assert!(captured.body.get("tool_choice").is_none());
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
