use std::{
    path::{Path, PathBuf},
    process::Command,
};

use agent::AgentDaemonConfiguration;
use tempfile::TempDir;

struct ConfigurationWriterSandbox {
    _directory: TempDir,
    ordinary_socket_path: PathBuf,
    meta_socket_path: PathBuf,
    database_path: PathBuf,
    output_path: PathBuf,
}

impl ConfigurationWriterSandbox {
    fn new() -> Self {
        let directory = TempDir::new().expect("tempdir");
        Self {
            ordinary_socket_path: directory.path().join("agent.sock"),
            meta_socket_path: directory.path().join("agent-meta.sock"),
            database_path: directory.path().join("agent.sema"),
            output_path: directory.path().join("agent.config.rkyv"),
            _directory: directory,
        }
    }

    fn request(&self) -> String {
        format!(
            "(AgentConfigurationWriteRequest ({} {} 384 {} [(ProviderSeed (criomos-local http://prometheus.goldragon.criome:11434/v1 gemma-4-26b-a4b (Gopass platform.deepseek.com/api-key)))] {}))",
            self.ordinary_socket_path.display(),
            self.meta_socket_path.display(),
            self.database_path.display(),
            self.output_path.display()
        )
    }

    fn output_path(&self) -> &Path {
        &self.output_path
    }
}

#[test]
fn configuration_writer_prebuilds_binary_archive_for_daemon_startup() {
    let sandbox = ConfigurationWriterSandbox::new();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-write-configuration"))
        .arg(sandbox.request())
        .output()
        .expect("run agent-write-configuration");
    assert!(
        output.status.success(),
        "writer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!(
            "(AgentConfigurationWritten {})",
            sandbox.output_path().display()
        )
    );

    let configuration =
        AgentDaemonConfiguration::from_binary_path(sandbox.output_path()).expect("read archive");
    assert_eq!(configuration.bootstrap_providers()[0].name, "criomos-local");
    assert!(matches!(
        configuration.bootstrap_providers()[0].secret_source,
        agent::registry::SecretSource::Gopass(_)
    ));
}
