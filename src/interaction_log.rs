//! Disabled-by-default provider interaction logging.
//!
//! The log is an operator-selected JSONL side file outside the agent redb. It
//! captures request and response context for prompt/schema repair while keeping
//! provider credentials out of the record.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{Map, Value};
use signal_agent::{CallRejection, CallRejectionReason};
use thiserror::Error;

use crate::config::ProviderInteractionLogging;
use crate::provider::{ProviderCall, ProviderCompletion, ProviderFailure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInteractionLog {
    destination: ProviderInteractionLogDestination,
}

impl ProviderInteractionLog {
    pub fn disabled() -> Self {
        Self {
            destination: ProviderInteractionLogDestination::Disabled,
        }
    }

    pub fn json_lines_at(path: impl Into<PathBuf>) -> Self {
        Self {
            destination: ProviderInteractionLogDestination::JsonLines(path.into()),
        }
    }

    pub fn from_configuration(
        database_path: &Path,
        logging: &ProviderInteractionLogging,
    ) -> Result<Self, ProviderInteractionLogError> {
        match logging {
            ProviderInteractionLogging::Disabled => Ok(Self::disabled()),
            ProviderInteractionLogging::JsonLines(path) => {
                let log_path = PathBuf::from(path.as_str());
                if log_path == database_path {
                    return Err(ProviderInteractionLogError::MainDatabasePath {
                        path: log_path.display().to_string(),
                    });
                }
                Ok(Self::json_lines_at(log_path))
            }
        }
    }

    pub fn is_disabled(&self) -> bool {
        matches!(
            self.destination,
            ProviderInteractionLogDestination::Disabled
        )
    }

    pub async fn record(
        &self,
        record: ProviderInteractionRecord,
    ) -> Result<(), ProviderInteractionLogError> {
        match &self.destination {
            ProviderInteractionLogDestination::Disabled => Ok(()),
            ProviderInteractionLogDestination::JsonLines(path) => {
                let path = path.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(ProviderInteractionLogError::from)?;
                    }
                    let mut file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .map_err(ProviderInteractionLogError::from)?;
                    serde_json::to_writer(&mut file, &record)
                        .map_err(ProviderInteractionLogError::from)?;
                    file.write_all(b"\n")
                        .map_err(ProviderInteractionLogError::from)
                })
                .await
                .map_err(|error| ProviderInteractionLogError::Join(error.to_string()))?
            }
        }
    }
}

impl Default for ProviderInteractionLog {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderInteractionLogDestination {
    Disabled,
    JsonLines(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderInteractionLogError {
    #[error("provider interaction log path must not be the agent database path: {path}")]
    MainDatabasePath { path: String },

    #[error("provider interaction log io failed: {0}")]
    Io(String),

    #[error("provider interaction log encode failed: {0}")]
    Encode(String),

    #[error("provider interaction log writer task failed: {0}")]
    Join(String),
}

impl From<std::io::Error> for ProviderInteractionLogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for ProviderInteractionLogError {
    fn from(error: serde_json::Error) -> Self {
        Self::Encode(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderValidationOutcome {
    NotRequired,
    ValidNota,
    InvalidNota { error: String },
}

impl ProviderValidationOutcome {
    pub fn not_required() -> Self {
        Self::NotRequired
    }

    pub fn valid_nota() -> Self {
        Self::ValidNota
    }

    pub fn invalid_nota(error: impl Into<String>) -> Self {
        Self::InvalidNota {
            error: error.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInteractionRecord {
    timestamp_unix_millis: u128,
    provider: ProviderMetadataLog,
    request: ProviderRequestLog,
    response: Option<ProviderResponseLog>,
    provider_result: ProviderResultLog,
    validation: ProviderValidationLog,
    daemon_outcome: ProviderDaemonOutcomeLog,
}

impl ProviderInteractionRecord {
    pub fn completed(
        call: &ProviderCall,
        completion: &ProviderCompletion,
        validation: ProviderValidationOutcome,
    ) -> Self {
        Self::new(
            call,
            ProviderResponseLog::from_completion(completion),
            ProviderResultLog::completed(completion),
            ProviderValidationLog::from_outcome(validation),
            ProviderDaemonOutcomeLog::Completed,
        )
    }

    pub fn rejected(
        call: &ProviderCall,
        failure: &ProviderFailure,
        rejection: &CallRejection,
    ) -> Self {
        Self::new(
            call,
            ProviderResponseLog::from_failure(failure),
            ProviderResultLog::failed(failure),
            ProviderValidationLog::NotReached,
            ProviderDaemonOutcomeLog::rejected(rejection),
        )
    }

    pub fn validation_rejected(
        call: &ProviderCall,
        completion: &ProviderCompletion,
        validation_error: &str,
        rejection: &CallRejection,
    ) -> Self {
        Self::new(
            call,
            ProviderResponseLog::from_completion(completion),
            ProviderResultLog::completed(completion),
            ProviderValidationLog::InvalidNota {
                error: validation_error.to_owned(),
            },
            ProviderDaemonOutcomeLog::rejected(rejection),
        )
    }

    fn new(
        call: &ProviderCall,
        response: Option<ProviderResponseLog>,
        provider_result: ProviderResultLog,
        validation: ProviderValidationLog,
        daemon_outcome: ProviderDaemonOutcomeLog,
    ) -> Self {
        Self {
            timestamp_unix_millis: ProviderInteractionClock::now_unix_millis(),
            provider: ProviderMetadataLog::from_call(call),
            request: ProviderRequestLog::from_call(call),
            response,
            provider_result,
            validation,
            daemon_outcome,
        }
    }
}

struct ProviderInteractionClock;

impl ProviderInteractionClock {
    fn now_unix_millis() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
struct ProviderMetadataLog {
    provider: String,
    endpoint: String,
    model: String,
    authorization: &'static str,
}

impl ProviderMetadataLog {
    fn from_call(call: &ProviderCall) -> Self {
        Self {
            provider: SecretRedactor::metadata(call.provider_name()),
            endpoint: SecretRedactor::url(call.endpoint()),
            model: SecretRedactor::metadata(call.model()),
            authorization: call.authorization().log_label(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ProviderRequestLog {
    url: String,
    headers: Map<String, Value>,
    body: Value,
}

impl ProviderRequestLog {
    fn from_call(call: &ProviderCall) -> Self {
        let mut headers = Map::new();
        if call.authorization().bearer_token().is_some() {
            headers.insert(
                "authorization".to_owned(),
                Value::String("<redacted>".to_owned()),
            );
        }
        Self {
            url: SecretRedactor::url(&call.chat_completions_url()),
            headers,
            body: SecretRedactor::json_value(call.request_body_value()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ProviderResponseLog {
    status: Option<u16>,
    body: Option<String>,
}

impl ProviderResponseLog {
    fn from_completion(completion: &ProviderCompletion) -> Option<Self> {
        if completion.response_status.is_none() && completion.response_body.is_none() {
            None
        } else {
            Some(Self {
                status: completion.response_status,
                body: completion
                    .response_body
                    .as_deref()
                    .map(SecretRedactor::body_text),
            })
        }
    }

    fn from_failure(failure: &ProviderFailure) -> Option<Self> {
        match failure {
            ProviderFailure::ProviderRejected {
                response_status,
                response_body,
                ..
            } => {
                if response_status.is_none() && response_body.is_none() {
                    None
                } else {
                    Some(Self {
                        status: *response_status,
                        body: response_body.as_deref().map(SecretRedactor::body_text),
                    })
                }
            }
            ProviderFailure::Unreachable(_) | ProviderFailure::OutputModeUnsupported => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
enum ProviderResultLog {
    Completed {
        completion_text: String,
        stop_reason: String,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
    },
    Failed {
        failure_kind: &'static str,
        detail: String,
    },
}

impl ProviderResultLog {
    fn completed(completion: &ProviderCompletion) -> Self {
        Self::Completed {
            completion_text: completion.text.clone(),
            stop_reason: completion.stop_reason.clone(),
            prompt_tokens: completion.prompt_tokens,
            completion_tokens: completion.completion_tokens,
        }
    }

    fn failed(failure: &ProviderFailure) -> Self {
        match failure {
            ProviderFailure::Unreachable(detail) => Self::Failed {
                failure_kind: "unreachable",
                detail: SecretRedactor::metadata(detail),
            },
            ProviderFailure::ProviderRejected { detail, .. } => Self::Failed {
                failure_kind: "provider_rejected",
                detail: SecretRedactor::metadata(detail),
            },
            ProviderFailure::OutputModeUnsupported => Self::Failed {
                failure_kind: "output_mode_unsupported",
                detail: "provider does not support the requested output mode".to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
enum ProviderValidationLog {
    NotRequired,
    ValidNota,
    InvalidNota { error: String },
    NotReached,
}

impl ProviderValidationLog {
    fn from_outcome(outcome: ProviderValidationOutcome) -> Self {
        match outcome {
            ProviderValidationOutcome::NotRequired => Self::NotRequired,
            ProviderValidationOutcome::ValidNota => Self::ValidNota,
            ProviderValidationOutcome::InvalidNota { error } => Self::InvalidNota { error },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
enum ProviderDaemonOutcomeLog {
    Completed,
    Rejected { reason: String, detail: String },
}

impl ProviderDaemonOutcomeLog {
    fn rejected(rejection: &CallRejection) -> Self {
        Self::Rejected {
            reason: ProviderRejectionReasonName::new(rejection.reason).into_string(),
            detail: SecretRedactor::metadata(rejection.detail.payload()),
        }
    }
}

struct ProviderRejectionReasonName {
    reason: CallRejectionReason,
}

impl ProviderRejectionReasonName {
    fn new(reason: CallRejectionReason) -> Self {
        Self { reason }
    }

    fn into_string(self) -> String {
        match self.reason {
            CallRejectionReason::NoProviderConfigured => "NoProviderConfigured",
            CallRejectionReason::DaemonUnconfigured => "DaemonUnconfigured",
            CallRejectionReason::ProviderUnreachable => "ProviderUnreachable",
            CallRejectionReason::ProviderRejected => "ProviderRejected",
            CallRejectionReason::OutputModeUnsupported => "OutputModeUnsupported",
            CallRejectionReason::InvalidNotaOutput => "InvalidNotaOutput",
        }
        .to_owned()
    }
}

struct SecretRedactor;

impl SecretRedactor {
    fn metadata(value: &str) -> String {
        if Self::credential_bearing_key(value) {
            "<redacted>".to_owned()
        } else {
            value.to_owned()
        }
    }

    fn url(value: &str) -> String {
        let Some((base, query)) = value.split_once('?') else {
            return value.to_owned();
        };
        let query = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, _)) if Self::credential_bearing_key(key) => {
                    format!("{key}=<redacted>")
                }
                _ => pair.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("&");
        format!("{base}?{query}")
    }

    fn body_text(value: &str) -> String {
        match serde_json::from_str::<Value>(value) {
            Ok(parsed) => Self::json_value(parsed).to_string(),
            Err(_) => value.to_owned(),
        }
    }

    fn json_value(value: Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.into_iter().map(Self::json_value).collect()),
            Value::Object(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| {
                        if Self::credential_bearing_key(&key) {
                            (key, Value::String("<redacted>".to_owned()))
                        } else {
                            (key, Self::json_value(value))
                        }
                    })
                    .collect(),
            ),
            other => other,
        }
    }

    fn credential_bearing_key(value: &str) -> bool {
        let lowered = value.to_ascii_lowercase();
        lowered.contains("authorization")
            || lowered.contains("api_key")
            || lowered.contains("apikey")
            || lowered.contains("bearer")
            || lowered.contains("credential")
            || lowered.contains("password")
            || lowered.contains("secret")
            || lowered.contains("token")
    }
}
