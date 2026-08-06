//! `agent-daemon` — the long-lived LLM-API-call daemon process.
//!
//! Takes exactly one argument: a binary rkyv startup configuration file (the
//! single-argument rule). `AgentDaemon` owns argument decoding, the two-tier
//! listener bind, and the request spine.

use agent::AgentDaemon;

fn main() -> std::process::ExitCode {
    AgentDaemon::run_to_exit_code()
}
