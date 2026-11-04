//! State used by the bundled CLI. Sharing the files keeps both clients in sync.

use std::path::PathBuf;
use std::process::Command;

/// The agent directory and session directory given to the companion daemon.
#[derive(Clone)]
pub struct CliData {
    agent: PathBuf,
    sessions: PathBuf,
}

impl CliData {
    #[must_use]
    pub fn new(agent: PathBuf) -> Self {
        let sessions = std::env::var_os(pa_daemon::paths::SESSION_DIR_ENV)
            .or_else(|| std::env::var_os("PRIME_AGENT_CODING_AGENT_SESSION_DIR"))
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .map_or_else(
                || agent.join("sessions"),
                |p| {
                    p.strip_prefix("~")
                        .ok()
                        .and_then(|rest| pa_types::platform::home_dir().map(|home| home.join(rest)))
                        .unwrap_or(p)
                },
            );
        Self { agent, sessions }
    }

    pub(super) fn configure(&self, command: &mut Command) {
        command
            .env(pa_types::platform::dirs::ENV_AGENT_DIR, &self.agent)
            .env(pa_daemon::paths::SESSION_DIR_ENV, &self.sessions);
    }

    pub(super) fn desktop_socket(&self) -> PathBuf {
        let dir = pa_daemon::socket::socket_dir();
        // Keep the original default endpoint, so an upgrade can reattach to existing turns.
        if pa_types::platform::home_dir()
            .is_some_and(|home| self.agent == home.join(pa_types::platform::dirs::CONFIG_DIR_NAME))
            && self.sessions == self.agent.join("sessions")
        {
            return dir.join(super::DESKTOP_DAEMON_SOCKET_NAME);
        }
        // One socket per store pair: changing profiles must never reuse the previous daemon's keys.
        let key = format!("{}\0{}", self.agent.display(), self.sessions.display());
        dir.join(format!(
            "desktop-{}.sock",
            pa_daemon::paths::hash_key(&key, 16)
        ))
    }
}
