//! Conditional writes to the CLI's shared credential store. Keep raw records inside the worker.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use pa_core::auth::{
    AuthCredential, AuthStorageBackend, AuthStorageData, FileAuthStorageBackend,
    GITHUB_COPILOT_PROVIDER_ID, OPENAI_CODEX_PROVIDER_ID, PRIME_AGENT_TRACES_PROVIDER_ID,
    PRIME_INFERENCE_PROVIDER_ID, PrimeTeamAssignment, SERPER_CREDENTIAL_ID,
};
use serde_json::Value;

/// Worker result. Intentionally has no Debug or Serialize implementation.
pub(crate) enum LoginCredential {
    Prime {
        api_key: String,
        team: PrimeTeamAssignment,
    },
    Provider(AuthCredential),
}

pub(super) fn is_model_provider(provider: &str) -> bool {
    !provider.trim().is_empty()
        && provider.trim() == provider
        && !provider.chars().any(char::is_control)
        && !matches!(
            provider,
            PRIME_AGENT_TRACES_PROVIDER_ID | SERPER_CREDENTIAL_ID
        )
        && !provider.starts_with("mcp:")
}

pub(super) fn is_oauth_only(provider: &str) -> bool {
    matches!(
        provider,
        GITHUB_COPILOT_PROVIDER_ID | OPENAI_CODEX_PROVIDER_ID
    )
}

pub(crate) fn supports_browser(provider: &str) -> bool {
    matches!(
        provider,
        PRIME_INFERENCE_PROVIDER_ID
            | OPENAI_CODEX_PROVIDER_ID
            | GITHUB_COPILOT_PROVIDER_ID
            | "anthropic"
            | "xai"
    )
}

fn literal_key(key: &str) -> bool {
    !key.trim().is_empty() && !key.trim_start().starts_with('!')
}

pub(super) fn editable_record(provider: &str, credential: Option<&AuthCredential>) -> bool {
    match credential {
        Some(AuthCredential::ApiKey { key, .. }) => !is_oauth_only(provider) && literal_key(key),
        Some(AuthCredential::Oauth { .. }) => {
            supports_browser(provider) && provider != PRIME_INFERENCE_PROVIDER_ID
        }
        _ => false,
    }
}

pub(super) fn read_document(agent_dir: &Path) -> Result<AuthStorageData> {
    // CLI updates replace the file atomically. A snapshot requires neither a lock nor a
    // profile initializer. FileAuthStorageBackend::read would create an empty auth.json.
    match std::fs::read(agent_dir.join("auth.json")) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| anyhow!("Cannot parse auth.json.")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(AuthStorageData::default()),
        Err(_) => bail!("Cannot read auth.json."),
    }
}

/// The exact target observed before sign-in. Never pass account tokens or this guard to views.
pub(crate) struct CredentialSlot {
    file: PathBuf,
    provider: String,
    previous: Option<Value>,
}

impl CredentialSlot {
    pub(crate) fn read(agent_dir: &Path, provider: &str) -> Result<Self> {
        ensure!(is_model_provider(provider), "This is not a model provider.");
        let mut doc = read_document(agent_dir)?;
        if doc.get(provider).is_some() {
            ensure!(
                editable_record(provider, doc.credential(provider).as_ref()),
                "Manage this credential in the CLI."
            );
        }
        Ok(Self {
            file: agent_dir.join("auth.json"),
            provider: provider.to_owned(),
            previous: doc.0.remove(provider),
        })
    }

    pub(crate) fn prime_team_id_for(&self, api_key: &str) -> Option<&str> {
        let previous = self.previous.as_ref()?;
        if self.provider != PRIME_INFERENCE_PROVIDER_ID || previous["key"].as_str() != Some(api_key)
        {
            return None;
        }
        previous.get("primeTeam")?.get("teamId")?.as_str()
    }

    pub(crate) fn save_checked(
        &self,
        credential: LoginCredential,
        begin_commit: impl FnOnce() -> bool,
    ) -> Result<()> {
        let value = match credential {
            LoginCredential::Prime { api_key, team } => {
                ensure!(
                    self.provider == PRIME_INFERENCE_PROVIDER_ID && literal_key(&api_key),
                    "Invalid Prime Inference credential."
                );
                let same_key = self
                    .previous
                    .as_ref()
                    .filter(|old| old["key"].as_str() == Some(api_key.as_str()));
                let team = match team {
                    PrimeTeamAssignment::Team(team) => serde_json::to_value(team)?,
                    PrimeTeamAssignment::PersonalAccount => Value::Null,
                    PrimeTeamAssignment::PreserveWhenKeyMatches => same_key
                        .and_then(|previous| previous.get("primeTeam"))
                        .cloned()
                        .unwrap_or(Value::Null),
                };
                let mut record = same_key
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "api_key", "key": api_key}));
                record["primeTeam"] = team;
                record
            }
            LoginCredential::Provider(credential) => {
                ensure!(
                    self.provider != PRIME_INFERENCE_PROVIDER_ID,
                    "Use the Prime Inference login."
                );
                ensure!(
                    editable_record(&self.provider, Some(&credential)),
                    "Unsupported provider credential."
                );
                if let AuthCredential::ApiKey { key, .. } = &credential {
                    self.require_api_key_slot()?;
                    if let Some(same) = self
                        .previous
                        .as_ref()
                        .filter(|old| old["key"].as_str() == Some(key.as_str()))
                    {
                        return self.replace(Some(same.clone()), begin_commit);
                    }
                }
                serde_json::to_value(credential)?
            }
        };
        self.replace(Some(value), begin_commit)
    }

    pub(crate) fn has_stored_api_key(&self) -> bool {
        !is_oauth_only(&self.provider)
            && self
                .previous
                .as_ref()
                .is_some_and(|stored| stored["type"] == "api_key")
    }

    pub(crate) fn remove_confirmed_key(&self, begin_commit: impl FnOnce() -> bool) -> Result<()> {
        ensure!(
            self.has_stored_api_key(),
            "No editable API key is stored for this provider."
        );
        self.replace(None, begin_commit)
    }

    pub(super) fn remove_api_key(&self) -> Result<()> {
        self.require_api_key_slot()?;
        self.replace(None, || true)
    }

    fn require_api_key_slot(&self) -> Result<()> {
        ensure!(
            !is_oauth_only(&self.provider)
                && self
                    .previous
                    .as_ref()
                    .is_none_or(|old| old["type"].as_str() == Some("api_key")),
            "Manage this sign-in through the provider login."
        );
        Ok(())
    }

    fn replace(&self, mut value: Option<Value>, begin_commit: impl FnOnce() -> bool) -> Result<()> {
        let mut gate = Some(begin_commit);
        FileAuthStorageBackend::new(&self.file).with_lock(&mut |current| {
            let current = current.context("Cannot read auth.json.")?;
            let mut doc: AuthStorageData =
                serde_json::from_str(&current).map_err(|_| anyhow!("Cannot parse auth.json."))?;
            ensure!(
                doc.get(&self.provider) == self.previous.as_ref(),
                "This provider's credential changed during sign-in. Start again."
            );
            let next = if value.as_ref() == self.previous.as_ref() {
                None
            } else {
                if let Some(value) = value.take() {
                    doc.0.insert(self.provider.clone(), value);
                } else {
                    doc.0.remove(&self.provider);
                }
                Some(format!("{}\n", serde_json::to_string_pretty(&doc)?))
            };
            // Cancellation and the right to commit race under the same login gate. Only the
            // winner may proceed. The CLI lock already protects both the comparison and write.
            ensure!(
                gate.take().is_some_and(|begin| begin()),
                "Sign-in was cancelled."
            );
            Ok(((), next))
        })
    }
}

#[cfg(test)]
mod tests;
