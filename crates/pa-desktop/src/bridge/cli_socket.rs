//! Attach to the daemon the CLI already owns. The installed Unix launcher and
//! the unwrapped binary have different default socket names (`install-rust.sh`).

use super::RawClient;
use anyhow::{Context as _, Result, anyhow};
use std::path::{Path, PathBuf};

fn candidates(explicit: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    let pinned = explicit.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("PRIME_AGENT_DAEMON_SOCKET")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    });
    if let Some(path) = pinned {
        let path = path
            .strip_prefix("~")
            .ok()
            .and_then(|rest| pa_types::platform::home_dir().map(|home| home.join(rest)))
            .unwrap_or(path);
        return (path, None);
    }
    let direct = pa_daemon::socket::default_daemon_socket_path();
    #[cfg(unix)]
    {
        // Match the installer's launcher (including Darwin's UNIX_PATH_MAX fallback).
        let tmp = std::env::var_os("TMPDIR")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        let relative = format!(
            "prime-agent-rust-{}/daemon.sock",
            pa_core::platform::process::current_user_id()
        );
        let installed = tmp.join(&relative);
        #[cfg(not(target_os = "linux"))]
        let installed = if std::os::unix::net::SocketAddr::from_pathname(&installed).is_ok() {
            installed
        } else {
            Path::new("/tmp").join(relative)
        };
        (installed, Some(direct))
    }
    #[cfg(not(unix))]
    {
        (direct, None)
    }
}

pub(super) fn preferred_cli_socket() -> PathBuf {
    candidates(None).0
}

pub(super) async fn connect_cli_socket(explicit: Option<&Path>) -> Result<(PathBuf, RawClient)> {
    let (path, alternate) = candidates(explicit);
    connect_existing(path, alternate).await
}

async fn connect_existing(
    path: PathBuf,
    alternate: Option<PathBuf>,
) -> Result<(PathBuf, RawClient)> {
    let first_error = match RawClient::connect(&path).await {
        Ok(client) => return Ok((path, client)),
        Err(error) => error,
    };
    if let Some(alternate) = alternate
        && first_error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                )
            })
    {
        let client = RawClient::connect(&alternate).await.map_err(|second| {
            anyhow!(
                "Could not attach to a CLI daemon. {}: {first_error:#}. {}: {second:#}.",
                path.display(),
                alternate.display()
            )
        })?;
        Ok((alternate, client))
    } else {
        Err(first_error).with_context(|| format!("Cannot connect to {}", path.display()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pa_types::daemon::{DAEMON_PROTOCOL_VERSION, DAEMON_SCHEMA_ID};
    use serde_json::json;
    use tokio::io::AsyncWriteExt as _;

    #[tokio::test]
    async fn shared_discovery_attaches_to_the_unwrapped_cli_when_only_it_is_running() {
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let installed = tmp.path().join("installed.sock");
        let direct = tmp.path().join("direct.sock");
        let listener = tokio::net::UnixListener::bind(&direct).unwrap();
        let greet = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let frame = json!({"type": "daemon_hello", "schemaId": DAEMON_SCHEMA_ID,
                "protocol": {"version": DAEMON_PROTOCOL_VERSION}});
            socket
                .write_all(format!("{frame}\n").as_bytes())
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(
            connect_existing(installed.clone(), Some(direct.clone())),
            greet
        );
        assert_eq!(outcome.unwrap().0, direct);
        assert!(!installed.exists());
    }
}
