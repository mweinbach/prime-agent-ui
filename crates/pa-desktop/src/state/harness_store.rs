//! Continual Harness (`memory`, `prompt`, `skill`, `subagent`, `refinements`)
//! and Provider Auth (`auth.json`) read/write facade backed by `pa-core`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use pa_core::auth::{
    AuthCredential, AuthStorage, AuthStorageBackend, AuthStorageData, FileAuthStorageBackend,
    GITHUB_COPILOT_PROVIDER_ID, NoOAuth, OPENAI_CODEX_PROVIDER_ID, PRIME_AGENT_TRACES_PROVIDER_ID,
    PRIME_INFERENCE_PROVIDER_ID, SERPER_CREDENTIAL_ID,
};
use pa_core::models::parse_models_config;
use pa_core::refinement::{
    HarnessEntry, HarnessScope, HarnessState, RefinementKind, RefinementResult,
    get_global_harness_state_dir, get_local_harness_state_dir, load_global_refinement_history,
    load_harness_state, update_harness_state,
};

use super::models::provider_display_name;

/// Redacted summary of the provider credentials shared with the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthItem {
    pub provider_id: String,
    pub display_name: String,
    pub configured: bool,
    pub auth_kind: String,
    pub hint: String,
    pub can_edit_api_key: bool,
    pub has_stored_api_key: bool,
}

/// Snapshot of continual harness entries & refinement history for the inspector.
#[derive(Debug, Clone, Default)]
pub struct HarnessSnapshot {
    pub global_dir: PathBuf,
    pub local_dir: Option<PathBuf>,
    pub global_entries: Vec<HarnessEntry>,
    pub local_entries: Vec<HarnessEntry>,
    pub history: Vec<RefinementResult>,
}

impl HarnessSnapshot {
    #[must_use]
    pub fn load(agent_dir: &Path, session_file: Option<&str>, session_id: Option<&str>) -> Self {
        let global_dir = get_global_harness_state_dir(agent_dir);
        let artifact_dir = resolve_session_artifact_dir(agent_dir, session_file, session_id);
        let local_dir = get_local_harness_state_dir(artifact_dir.as_deref());

        let global_state = load_harness_state(&global_dir, HarnessScope::Global);
        let global_entries = flatten_entries(&global_state, HarnessScope::Global);

        let local_entries = local_dir
            .as_deref()
            .map(|dir| {
                let state = load_harness_state(dir, HarnessScope::Local);
                flatten_entries(&state, HarnessScope::Local)
            })
            .unwrap_or_default();

        let mut history = load_global_refinement_history(&global_dir);
        if let Some(dir) = local_dir.as_deref() {
            let mut local_hist = load_global_refinement_history(dir);
            local_hist.append(&mut history);
            history = local_hist;
        }
        history.truncate(25);

        Self {
            global_dir,
            local_dir,
            global_entries,
            local_entries,
            history,
        }
    }

    #[must_use]
    pub fn entries_for(&self, scope: HarnessScope) -> &[HarnessEntry] {
        match scope {
            HarnessScope::Global => &self.global_entries,
            HarnessScope::Local => &self.local_entries,
        }
    }
}

fn flatten_entries(state: &HarnessState, scope: HarnessScope) -> Vec<HarnessEntry> {
    let mut rows: Vec<HarnessEntry> = Vec::new();
    for entries in state.entries.values() {
        for entry in entries.values() {
            let mut item = entry.clone();
            item.scope = Some(scope);
            rows.push(item);
        }
    }
    rows.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

/// Resolve a persisted session's artifact directory (`<agent_dir>/session-artifacts/<id>`),
/// matching `pa_core::session_engine::harness_digest::session_artifact_dir_for_log`.
#[must_use]
pub fn resolve_session_artifact_dir(
    agent_dir: &Path,
    session_file: Option<&str>,
    session_id: Option<&str>,
) -> Option<PathBuf> {
    if let Some(sf) = session_file.filter(|s| !s.is_empty())
        && let Some(dir) =
            pa_core::session_engine::harness_digest::session_artifact_dir_for_log(Path::new(sf))
    {
        return Some(dir);
    }
    let sid = session_id?.trim();
    if matches!(sid, "" | "." | "..") || sid.contains(['/', '\\', ':']) {
        return None;
    }
    Some(agent_dir.join("session-artifacts").join(sid))
}

/// Parameters for creating or updating one continual harness entry.
pub struct HarnessEntryDraft<'a> {
    pub scope: HarnessScope,
    pub kind: RefinementKind,
    pub id_override: Option<&'a str>,
    pub title: &'a str,
    pub content: &'a str,
    pub category_path: Option<&'a str>,
}

/// Create or update one continual harness entry under lock (`{file}.lock` + `0o600` atomic write).
///
/// # Errors
///
/// Returns an error if the target scope directory is unavailable or the locked write fails.
pub fn upsert_harness_entry(
    agent_dir: &Path,
    session_file: Option<&str>,
    session_id: Option<&str>,
    draft: &HarnessEntryDraft<'_>,
) -> Result<HarnessEntry> {
    let target_dir = resolve_scope_dir(agent_dir, session_file, session_id, draft.scope)?;
    let now = now_iso8601();
    let clean_title = draft.title.trim();
    let slug_id = draft
        .id_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| slugify_id(clean_title), str::to_string);
    let default_path = match draft.kind {
        RefinementKind::Prompt => "policy",
        RefinementKind::Memory
        | RefinementKind::Skill
        | RefinementKind::Subagent
        | RefinementKind::Factory => "general",
    };
    let entry_path = draft
        .category_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_path)
        .to_string();

    let (saved, _) = update_harness_state(&target_dir, draft.scope, |state| {
        let bucket = state.entries.entry(draft.kind).or_default();
        if let Some(existing) = bucket.get_mut(&slug_id) {
            existing.title = clean_title.to_string();
            existing.content = draft.content.trim().to_string();
            existing.path = entry_path;
            existing.updated_at = now;
            existing.version = existing.version.saturating_add(1);
            existing.clone()
        } else {
            let fresh = HarnessEntry {
                id: slug_id.clone(),
                kind: draft.kind,
                title: clean_title.to_string(),
                content: draft.content.trim().to_string(),
                path: entry_path,
                scope: Some(draft.scope),
                reference: serde_json::Map::new(),
                arguments: serde_json::Map::new(),
                metadata: serde_json::Map::new(),
                source: "agent".to_string(),
                created_at: now.clone(),
                updated_at: now,
                version: 1,
            };
            bucket.insert(slug_id, fresh.clone());
            fresh
        }
    })?;
    Ok(saved)
}

/// Delete one continual harness entry by `(scope, kind, id)`.
///
/// # Errors
///
/// Returns an error if the target scope directory is unavailable or the locked write fails.
pub fn delete_harness_entry(
    agent_dir: &Path,
    session_file: Option<&str>,
    session_id: Option<&str>,
    scope: HarnessScope,
    kind: RefinementKind,
    id: &str,
) -> Result<bool> {
    let target_dir = resolve_scope_dir(agent_dir, session_file, session_id, scope)?;
    let (removed, _) = update_harness_state(&target_dir, scope, |state| {
        state
            .entries
            .get_mut(&kind)
            .and_then(|bucket| bucket.remove(id))
            .is_some()
    })?;
    Ok(removed)
}

fn resolve_scope_dir(
    agent_dir: &Path,
    session_file: Option<&str>,
    session_id: Option<&str>,
    scope: HarnessScope,
) -> Result<PathBuf> {
    match scope {
        HarnessScope::Global => Ok(get_global_harness_state_dir(agent_dir)),
        HarnessScope::Local => {
            let artifact_dir = resolve_session_artifact_dir(agent_dir, session_file, session_id)
                .context("local continual harness requires an active persisted session")?;
            get_local_harness_state_dir(Some(&artifact_dir))
                .context("could not resolve local continual harness directory")
        }
    }
}

#[must_use]
pub fn slugify_id(title: &str) -> String {
    let mut slug = String::new();
    let mut prev_underscore = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !slug.is_empty() {
            slug.push('_');
            prev_underscore = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        format!("entry_{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
    } else {
        trimmed.to_string()
    }
}

#[must_use]
pub fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "2026-01-01T00:00:00Z".to_string())
}

const DEFAULT_PROVIDERS: &[&str] = &[
    PRIME_INFERENCE_PROVIDER_ID,
    OPENAI_CODEX_PROVIDER_ID,
    "anthropic",
    "google",
    "openai",
    "mistral",
    "openrouter",
    GITHUB_COPILOT_PROVIDER_ID,
    "xai",
];

fn is_model_provider(provider_id: &str) -> bool {
    !provider_id.is_empty()
        && !matches!(
            provider_id,
            SERPER_CREDENTIAL_ID | PRIME_AGENT_TRACES_PROVIDER_ID
        )
        && !provider_id.starts_with("mcp:")
}

fn is_oauth_only(provider_id: &str) -> bool {
    matches!(
        provider_id,
        OPENAI_CODEX_PROVIDER_ID | GITHUB_COPILOT_PROVIDER_ID
    )
}

/// Read credential metadata without creating files, resolving command keys, or refreshing OAuth.
#[must_use]
pub fn load_provider_auth_items(agent_dir: &Path) -> Vec<ProviderAuthItem> {
    // The shared writer replaces auth.json atomically. A read needs no lock file
    // or initializer, unlike the CLI auth backend's read operation.
    let doc: Option<AuthStorageData> = match std::fs::read(agent_dir.join("auth.json")) {
        Ok(raw) => serde_json::from_slice(&raw).ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Some(AuthStorageData::default())
        }
        Err(_) => None,
    };
    let models = std::fs::read_to_string(agent_dir.join("models.json"))
        .ok()
        .and_then(|raw| parse_models_config(&raw).ok());
    // Only ask the CLI resolver about the environment. Giving it stored or
    // models.json keys here could execute a credential command just to show Settings.
    let env = AuthStorage::in_memory(&AuthStorageData::default(), Arc::new(NoOAuth));
    let additional_ids: BTreeSet<&str> = doc
        .iter()
        .flat_map(|doc| doc.0.keys().map(String::as_str))
        .chain(
            models
                .iter()
                .flat_map(|config| config.providers.keys().map(String::as_str)),
        )
        .filter(|id| is_model_provider(id) && !DEFAULT_PROVIDERS.contains(id))
        .collect();

    DEFAULT_PROVIDERS
        .iter()
        .copied()
        .chain(additional_ids)
        .map(|provider_id| {
            let stored = doc.as_ref().and_then(|doc| doc.credential(provider_id));
            let has_record = doc
                .as_ref()
                .is_some_and(|doc| doc.get(provider_id).is_some());
            let has_stored_api_key = matches!(stored, Some(AuthCredential::ApiKey { .. }));
            let config = models
                .as_ref()
                .and_then(|models| models.providers.get(provider_id));
            let config_has_auth = config.is_some_and(|config| {
                config.api_key.is_some() || config.headers.is_some() || config.auth_header.is_some()
            });
            let display_name = config
                .and_then(|config| config.name.as_deref())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| match provider_id {
                    GITHUB_COPILOT_PROVIDER_ID => "GitHub Copilot",
                    _ => provider_display_name(provider_id),
                });
            let environment = env.get_auth_status(provider_id).label.map(|label| {
                let hint = if label == "ambient credentials" {
                    "From this app's environment".to_string()
                } else {
                    format!("From ${label} in this app's environment")
                };
                (true, "Environment", hint)
            });
            let saved = match stored {
                Some(AuthCredential::ApiKey { .. }) => {
                    Some((true, "API key", "Saved by Prime Agent".to_string()))
                }
                Some(AuthCredential::Oauth { .. }) => {
                    Some((true, "OAuth", "Manage this sign-in in the CLI".to_string()))
                }
                Some(AuthCredential::McpStaticToken { .. }) => Some((
                    true,
                    "Saved token",
                    "Manage this credential in the CLI".to_string(),
                )),
                None => None,
            };
            // The CLI reads PRIME_API_KEY before checking the saved Prime Inference key.
            let preferred = if provider_id == PRIME_INFERENCE_PROVIDER_ID {
                environment.or(saved)
            } else {
                saved.or(environment)
            };
            let (configured, auth_kind, hint) = preferred.unwrap_or_else(|| {
                if config_has_auth {
                    (
                        true,
                        "Models config",
                        "Configured in models.json".to_string(),
                    )
                } else if has_record {
                    (
                        false,
                        "Saved credential",
                        "Manage this credential in the CLI".to_string(),
                    )
                } else if doc.is_none() {
                    (false, "Unavailable", "Cannot read auth.json".to_string())
                } else if is_oauth_only(provider_id) || provider_id == PRIME_INFERENCE_PROVIDER_ID {
                    (false, "Not configured", "Use /login in the CLI".to_string())
                } else {
                    (false, "Not configured", "No saved credential".to_string())
                }
            });
            ProviderAuthItem {
                provider_id: provider_id.to_string(),
                display_name: display_name.to_string(),
                configured,
                auth_kind: auth_kind.to_string(),
                hint,
                can_edit_api_key: doc.is_some()
                    && !is_oauth_only(provider_id)
                    && (!has_record || has_stored_api_key),
                has_stored_api_key,
            }
        })
        .collect()
}

/// Save or remove a provider API key in the shared `0o600` auth store.
///
/// # Errors
///
/// Returns an error if the target is not an API-key provider, contains a non-key
/// record, or the locked update fails.
pub fn set_provider_api_key(agent_dir: &Path, provider_id: &str, api_key: &str) -> Result<()> {
    if !is_model_provider(provider_id) || is_oauth_only(provider_id) {
        bail!("Manage this credential in the CLI");
    }
    let backend = FileAuthStorageBackend::new(agent_dir.join("auth.json"));
    let trimmed = api_key.trim();
    backend.with_lock(&mut |current: Option<String>| {
        let raw = current.context("Cannot read auth.json; no credentials were changed")?;
        let mut doc: AuthStorageData = serde_json::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("Cannot parse auth.json; no credentials were changed"))?;
        // Recheck under the lock: the CLI may have signed in after Settings was opened.
        if doc.get(provider_id).is_some() {
            let Some(AuthCredential::ApiKey { key, .. }) = doc.credential(provider_id) else {
                bail!("Manage this saved credential in the CLI; no credentials were changed");
            };
            if !trimmed.is_empty() && key == trimmed {
                return Ok(((), None));
            }
        } else if trimmed.is_empty() {
            return Ok(((), None));
        }
        if trimmed.is_empty() {
            doc.remove(provider_id);
        } else {
            doc.insert(
                provider_id,
                &AuthCredential::ApiKey {
                    key: trimmed.to_string(),
                    prime_team: None,
                },
            );
        }
        let serialized = serde_json::to_string_pretty(&doc)?;
        Ok(((), Some(format!("{serialized}\n"))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;

    #[test]
    fn settings_find_shared_credentials_without_exposing_or_resolving_them() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("key-command-ran");
        fs::write(
            dir.path().join("auth.json"),
            json!({
                "openai-codex": {"type":"oauth", "access":"fixture-access", "refresh":"fixture-refresh",
                    "expires":4_102_444_800_000_u64, "accountId":"fixture-account"},
                "custom-saved": {"type":"api_key", "key":format!("!touch '{}'; echo fixture-saved-key", marker.display())},
                "mcp:tool": {"type":"mcp_static_token", "bearer":"fixture-mcp"}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            dir.path().join("models.json"),
            json!({"providers": {"custom-config": {"name":"Fixture provider", "apiKey":"fixture-config-key",
                "baseUrl":"http://127.0.0.1:9/v1", "api":"openai-completions", "models":[{"id":"test"}]}}})
            .to_string(),
        )
        .unwrap();

        let rows = load_provider_auth_items(dir.path());
        let row = |id| rows.iter().find(|item| item.provider_id == id).unwrap();
        assert_eq!(
            row("openai-codex"),
            &ProviderAuthItem {
                provider_id: "openai-codex".to_string(),
                display_name: "OpenAI Codex".to_string(),
                configured: true,
                auth_kind: "OAuth".to_string(),
                hint: "Manage this sign-in in the CLI".to_string(),
                can_edit_api_key: false,
                has_stored_api_key: false,
            }
        );
        assert!(row("custom-saved").has_stored_api_key);
        assert_eq!(row("custom-config").auth_kind, "Models config");
        assert!(!rows.iter().any(|item| item.provider_id == "mcp:tool"));
        let rendered = format!("{rows:?}");
        for secret in [
            "fixture-access",
            "fixture-refresh",
            "fixture-account",
            "fixture-mcp",
            "fixture-saved-key",
            "fixture-config-key",
        ] {
            assert!(!rendered.contains(secret));
        }
        assert!(!marker.exists());
    }

    #[test]
    fn shared_key_edits_preserve_oauth_other_records_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = dir.path().join("auth.json");
        let settings_path = dir.path().join("settings.json");
        let settings = r#"{"theme":"light","defaultModel":"fixture"}"#;
        let original = json!({
            "anthropic":{"type":"oauth", "access":"fixture-access", "refresh":"fixture-refresh", "expires":4_102_444_800_000_u64},
            "prime-inference":{"type":"api_key", "key":"fixture-prime-key", "primeTeam":null},
            "unknown":{"type":"future-auth", "data":[1, 2, 3]}
        });
        fs::write(&auth_path, original.to_string()).unwrap();
        fs::write(&settings_path, settings).unwrap();
        let read_auth =
            || serde_json::from_str::<Value>(&fs::read_to_string(&auth_path).unwrap()).unwrap();

        assert!(set_provider_api_key(dir.path(), "anthropic", "new-key").is_err());
        assert!(set_provider_api_key(dir.path(), "anthropic", "").is_err());
        set_provider_api_key(dir.path(), "prime-inference", "fixture-prime-key").unwrap();
        assert_eq!(read_auth(), original);
        set_provider_api_key(dir.path(), "openai", " fixture-openai-key ").unwrap();
        let mut with_new_key = original.clone();
        with_new_key["openai"] = json!({"type":"api_key", "key":"fixture-openai-key"});
        assert_eq!(read_auth(), with_new_key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        set_provider_api_key(dir.path(), "openai", "").unwrap();
        assert_eq!(read_auth(), original);
        assert_eq!(fs::read_to_string(settings_path).unwrap(), settings);
    }

    #[test]
    fn opening_settings_does_not_create_a_store_but_first_save_does() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        let rows = load_provider_auth_items(&agent_dir);
        assert!(
            rows.iter()
                .any(|item| item.provider_id == "openai" && item.can_edit_api_key)
        );
        assert!(!agent_dir.exists());

        set_provider_api_key(&agent_dir, "openai", "fixture-first-key").unwrap();
        let auth: Value =
            serde_json::from_slice(&fs::read(agent_dir.join("auth.json")).unwrap()).unwrap();
        assert_eq!(
            auth,
            json!({"openai": {"type":"api_key", "key":"fixture-first-key"}})
        );
    }

    #[test]
    fn a_key_update_never_replaces_an_unreadable_auth_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let damaged = "{partial";
        std::fs::write(&path, damaged).unwrap();
        assert!(set_provider_api_key(dir.path(), "google", "test").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), damaged);
    }
}
