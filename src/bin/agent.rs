//! `agent` — the thin CLI client for the agent daemon.
//!
//! Takes exactly one NOTA argument naming a `signal_agent::Input` request,
//! sends it to the daemon over `AGENT_SOCKET`, and prints the reply as NOTA.

use agent::client::CommandLine;

fn main() -> std::process::ExitCode {
    match CommandLine::from_environment().run(std::io::stdout().lock()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("(AgentClientError [{error}])");
            std::process::ExitCode::from(2)
        }
    }
}
