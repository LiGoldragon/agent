//! `agent-write-configuration` — encode typed NOTA into daemon startup rkyv.
//!
//! This is the deploy/bootstrap text edge. `agent-daemon` itself still takes
//! exactly one binary rkyv configuration file and never parses NOTA.

use std::{
    fs,
    path::{Path, PathBuf},
};

use agent::{AgentDaemonConfiguration, ConfigurationError, ProviderSeed as RuntimeProviderSeed};
use nota_next::{Delimiter, NotaBlock, NotaDecode, NotaDecodeError, NotaEncode, NotaSource};
use thiserror::Error;
use triad_runtime::{ArgumentError, ComponentArgument, ComponentCommand};

fn main() {
    if let Err(error) = AgentConfigurationWriterCli::from_environment().run() {
        eprintln!("agent-write-configuration: {error}");
        std::process::exit(1);
    }
}

struct AgentConfigurationWriterCli {
    command: ComponentCommand,
}

struct AgentConfigurationWriterInputSource {
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaEncode)]
struct AgentConfigurationWriteRequest {
    ordinary_socket_path: ConfigurationWriterPath,
    meta_socket_path: ConfigurationWriterPath,
    meta_socket_mode: ConfigurationWriterSocketMode,
    database_path: ConfigurationWriterPath,
    bootstrap_providers: Vec<ProviderSeed>,
    output_path: ConfigurationWriterPath,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterPath(String);

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterSocketMode(u32);

#[derive(Debug, Clone, PartialEq, Eq, NotaEncode)]
struct ProviderSeed {
    name: ConfigurationWriterProviderName,
    endpoint: ConfigurationWriterEndpoint,
    default_model: ConfigurationWriterModelName,
    api_key_handle: ConfigurationWriterApiKeyHandle,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterProviderName(String);

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterEndpoint(String);

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterModelName(String);

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
struct ConfigurationWriterApiKeyHandle(String);

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentConfigurationWriteOutput {
    output_path: ConfigurationWriterPath,
}

impl AgentConfigurationWriterCli {
    fn from_environment() -> Self {
        Self {
            command: ComponentCommand::from_environment(),
        }
    }

    fn run(&self) -> Result<(), AgentConfigurationWriterCliError> {
        let source = self.source()?;
        let request = source.parse_request()?;
        let output = request.write()?;
        println!("{}", output.to_nota());
        Ok(())
    }

    fn source(
        &self,
    ) -> Result<AgentConfigurationWriterInputSource, AgentConfigurationWriterCliError> {
        match self.command.nota_argument()? {
            ComponentArgument::InlineNota(argument) => Ok(
                AgentConfigurationWriterInputSource::new(argument.into_string()),
            ),
            ComponentArgument::NotaFile(file) => {
                let path = file.into_path();
                fs::read_to_string(&path)
                    .map(AgentConfigurationWriterInputSource::new)
                    .map_err(|source| AgentConfigurationWriterCliError::ReadNotaFile {
                        path,
                        source,
                    })
            }
            ComponentArgument::SignalFile(file) => {
                Err(AgentConfigurationWriterCliError::UnsupportedSignalFile {
                    path: file.into_path(),
                })
            }
        }
    }
}

impl AgentConfigurationWriterInputSource {
    fn new(text: String) -> Self {
        Self { text }
    }

    fn parse_request(&self) -> Result<AgentConfigurationWriteRequest, NotaDecodeError> {
        NotaSource::new(&self.text).parse()
    }
}

impl AgentConfigurationWriteRequest {
    fn write(self) -> Result<AgentConfigurationWriteOutput, AgentConfigurationWriterCliError> {
        let output_path = self.output_path.clone();
        let configuration = self.configuration();
        configuration
            .write_binary_file(output_path.as_path())
            .map_err(AgentConfigurationWriterCliError::Archive)?;
        Ok(AgentConfigurationWriteOutput { output_path })
    }

    fn configuration(self) -> AgentDaemonConfiguration {
        AgentDaemonConfiguration::new(
            self.ordinary_socket_path.into_string(),
            self.meta_socket_path.into_string(),
            self.meta_socket_mode.into_mode(),
            self.database_path.into_string(),
            self.bootstrap_providers
                .into_iter()
                .map(ProviderSeed::into_runtime_provider_seed)
                .collect(),
        )
    }
}

impl NotaDecode for AgentConfigurationWriteRequest {
    fn from_nota_block(block: &nota_next::Block) -> Result<Self, NotaDecodeError> {
        let body = NotaBlock::new(block)
            .expect_body(Delimiter::Parenthesis, "AgentConfigurationWriteRequest")?;
        let objects = body.root_objects();
        if objects.len() != 7 {
            return Err(NotaDecodeError::ExpectedRootCount {
                type_name: "AgentConfigurationWriteRequest",
                expected: 7,
                found: objects.len(),
            });
        }
        match objects[0].demote_to_string() {
            Some("AgentConfigurationWriteRequest") => {}
            Some(variant) => {
                return Err(NotaDecodeError::UnknownVariant {
                    enum_name: "AgentConfigurationWriteRequest",
                    variant: variant.to_owned(),
                });
            }
            None => {
                return Err(NotaDecodeError::ExpectedAtom {
                    type_name: "AgentConfigurationWriteRequest",
                });
            }
        }
        Ok(Self {
            ordinary_socket_path: ConfigurationWriterPath::from_nota_block(&objects[1])?,
            meta_socket_path: ConfigurationWriterPath::from_nota_block(&objects[2])?,
            meta_socket_mode: ConfigurationWriterSocketMode::from_nota_block(&objects[3])?,
            database_path: ConfigurationWriterPath::from_nota_block(&objects[4])?,
            bootstrap_providers: Vec::<ProviderSeed>::from_nota_block(&objects[5])?,
            output_path: ConfigurationWriterPath::from_nota_block(&objects[6])?,
        })
    }
}

impl NotaEncode for AgentConfigurationWriteOutput {
    fn to_nota(&self) -> String {
        Delimiter::Parenthesis.wrap([
            String::from("AgentConfigurationWritten"),
            self.output_path.to_nota(),
        ])
    }
}

impl ConfigurationWriterPath {
    fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    fn into_string(self) -> String {
        self.0
    }
}

impl ConfigurationWriterSocketMode {
    fn into_mode(self) -> u32 {
        self.0
    }
}

impl ProviderSeed {
    fn into_runtime_provider_seed(self) -> RuntimeProviderSeed {
        RuntimeProviderSeed::new(
            self.name.into_string(),
            self.endpoint.into_string(),
            self.default_model.into_string(),
            self.api_key_handle.into_string(),
        )
    }
}

impl NotaDecode for ProviderSeed {
    fn from_nota_block(block: &nota_next::Block) -> Result<Self, NotaDecodeError> {
        let body = NotaBlock::new(block).expect_body(Delimiter::Parenthesis, "ProviderSeed")?;
        let objects = body.root_objects();
        if objects.len() != 5 {
            return Err(NotaDecodeError::ExpectedRootCount {
                type_name: "ProviderSeed",
                expected: 5,
                found: objects.len(),
            });
        }
        match objects[0].demote_to_string() {
            Some("ProviderSeed") => {}
            Some(variant) => {
                return Err(NotaDecodeError::UnknownVariant {
                    enum_name: "ProviderSeed",
                    variant: variant.to_owned(),
                });
            }
            None => {
                return Err(NotaDecodeError::ExpectedAtom {
                    type_name: "ProviderSeed",
                });
            }
        }
        Ok(Self {
            name: ConfigurationWriterProviderName::from_nota_block(&objects[1])?,
            endpoint: ConfigurationWriterEndpoint::from_nota_block(&objects[2])?,
            default_model: ConfigurationWriterModelName::from_nota_block(&objects[3])?,
            api_key_handle: ConfigurationWriterApiKeyHandle::from_nota_block(&objects[4])?,
        })
    }
}

impl ConfigurationWriterProviderName {
    fn into_string(self) -> String {
        self.0
    }
}

impl ConfigurationWriterEndpoint {
    fn into_string(self) -> String {
        self.0
    }
}

impl ConfigurationWriterModelName {
    fn into_string(self) -> String {
        self.0
    }
}

impl ConfigurationWriterApiKeyHandle {
    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Error)]
enum AgentConfigurationWriterCliError {
    #[error(transparent)]
    Argument(#[from] ArgumentError),

    #[error("read NOTA file {}: {source}", path.display())]
    ReadNotaFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(
        "signal-encoded configuration writer requests are not implemented yet for {}",
        path.display()
    )]
    UnsupportedSignalFile { path: PathBuf },

    #[error(transparent)]
    Decode(#[from] NotaDecodeError),

    #[error(transparent)]
    Archive(#[from] ConfigurationError),
}
