//! Account-scoped Codex and Gemini catalog resolution.

use pa_models::google::GoogleCredentials;
use pa_models::openai_codex::CodexCredentials;
use pa_types::ai::Model;

use crate::auth::{AuthCredential, AuthSource, AuthStorage, OPENAI_CODEX_PROVIDER_ID};

use super::registry::ModelRegistry;

const GOOGLE_PROVIDER_ID: &str = "google";

impl ModelRegistry {
    pub(super) fn merge_authorized_provider_catalogs(&self, built_in: &mut Vec<Model>) {
        if let Some(authorized) = self
            .codex_credentials()
            .and_then(|credentials| self.catalog.codex().get(&credentials))
        {
            built_in.retain(|model| model.provider != OPENAI_CODEX_PROVIDER_ID);
            built_in.extend(authorized);
        }
        if let Some(authorized) = self
            .google_credentials()
            .and_then(|credentials| self.catalog.google().get(&credentials))
        {
            built_in.retain(|model| model.provider != GOOGLE_PROVIDER_ID);
            built_in.extend(authorized);
        }
    }

    pub(super) async fn refresh_authorized_provider_catalogs(&mut self, force: bool) {
        if pa_models::offline::is_catalog_offline() {
            return;
        }
        if self.codex_credentials().is_none()
            && self
                .auth
                .get_auth_status(GOOGLE_PROVIDER_ID)
                .source
                .is_none()
        {
            return;
        }
        let mut auth = std::mem::replace(
            &mut self.auth,
            AuthStorage::in_memory_without_env(
                &crate::auth::types::AuthStorageData::default(),
                std::sync::Arc::new(crate::auth::manager::NoOAuth),
            ),
        );
        let (auth, codex_credentials, google_credentials) =
            tokio::task::spawn_blocking(move || {
                let codex_credentials = if auth.get_auth_status(OPENAI_CODEX_PROVIDER_ID).source
                    == Some(AuthSource::Stored)
                    && auth.get_api_key(OPENAI_CODEX_PROVIDER_ID).is_some()
                {
                    match auth.get_all().credential(OPENAI_CODEX_PROVIDER_ID) {
                        Some(AuthCredential::Oauth {
                            access,
                            account_id: Some(account_id),
                            ..
                        }) => Some(CodexCredentials {
                            access_token: access,
                            account_id,
                        }),
                        _ => None,
                    }
                } else {
                    None
                };
                let google_credentials = auth
                    .get_api_key(GOOGLE_PROVIDER_ID)
                    .map(|api_key| GoogleCredentials { api_key });
                (auth, codex_credentials, google_credentials)
            })
            .await
            .expect("credential refresh task ran");
        self.auth = auth;
        if let Some(credentials) = codex_credentials {
            self.catalog.codex().refresh(&credentials, force).await;
        }
        if let Some(credentials) = google_credentials {
            self.catalog.google().refresh(&credentials, force).await;
        }
    }

    /// Cache-only credential lookup. In particular, an expired OAuth token
    /// must not trigger a network refresh on the model picker's paint path.
    pub(super) fn codex_credentials(&self) -> Option<CodexCredentials> {
        if self.auth.get_auth_status(OPENAI_CODEX_PROVIDER_ID).source != Some(AuthSource::Stored) {
            return None;
        }
        let AuthCredential::Oauth {
            access,
            account_id: Some(account_id),
            ..
        } = self.auth.get_all().credential(OPENAI_CODEX_PROVIDER_ID)?
        else {
            return None;
        };
        Some(CodexCredentials {
            access_token: access,
            account_id,
        })
    }

    /// Cache-only Gemini API key lookup. Shell-command keys (`!cmd`) are
    /// resolved only on the awaited background refresh path.
    pub(super) fn google_credentials(&self) -> Option<GoogleCredentials> {
        let status = self.auth.get_auth_status(GOOGLE_PROVIDER_ID);
        let api_key = match status.source {
            Some(AuthSource::Stored) => match self.auth.get_all().credential(GOOGLE_PROVIDER_ID)? {
                AuthCredential::ApiKey { key, .. } if !key.starts_with('!') => {
                    crate::auth::resolve_config_value::resolve_config_value(&key)
                }
                _ => None,
            },
            Some(AuthSource::Environment) => {
                pa_ai::env_api_keys::get_env_api_key(GOOGLE_PROVIDER_ID)
            }
            _ => None,
        }?;
        Some(GoogleCredentials { api_key })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::auth::manager::{AuthStorage, NoOAuth};
    use crate::auth::types::AuthStorageData;

    use super::*;

    #[test]
    fn cached_codex_and_google_entitlements_replace_builtins_per_credential() {
        let dir = tempfile::tempdir().unwrap();
        let models_path = dir.path().join("models.json");
        std::fs::write(
            &models_path,
            serde_json::json!({"providers": {"openai-codex": {
                "modelOverrides": {"account-only": {"name": "My Codex model"}},
                "models": [{"id": "explicit-custom"}]
            }}})
            .to_string(),
        )
        .unwrap();
        let codex_credentials = CodexCredentials {
            access_token: "account-a-token".to_string(),
            account_id: "account-a".to_string(),
        };
        let google_credentials = GoogleCredentials {
            api_key: "google-key-a".to_string(),
        };
        let catalog = super::super::catalog_chain::catalog_for(Some(&models_path));
        let codex_scope = catalog.codex().scope_for(&codex_credentials);
        let google_scope = catalog.google().scope_for(&google_credentials);
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path()
                .join(format!("models/openai-codex-models-{codex_scope}.json")),
            serde_json::json!({
                "url": format!(
                    "https://chatgpt.com/backend-api/codex/models?client_version={}",
                    pa_ai::oauth::CODEX_CLIENT_VERSION
                ),
                "scope": codex_scope,
                "fetchedAt": 1,
                "payload": {"models": [{
                    "slug": "account-only",
                    "display_name": "Account model",
                    "visibility": "list",
                    "supported_in_api": true,
                    "context_window": 200_000,
                    "input_modalities": ["text"],
                    "supported_reasoning_levels": [{"effort": "high", "description": "High"}]
                }]},
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path()
                .join(format!("models/google-models-{google_scope}.json")),
            serde_json::json!({
                "url": "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000",
                "scope": google_scope,
                "fetchedAt": 1,
                "payload": {"models": [{
                    "name": "models/gemini-2.5-custom-exp",
                    "displayName": "Gemini 2.5 Custom Exp",
                    "inputTokenLimit": 1_048_576,
                    "outputTokenLimit": 65_536,
                    "supportedGenerationMethods": ["generateContent"],
                    "thinking": true
                }]},
            })
            .to_string(),
        )
        .unwrap();
        // An expired token may serve a known account's snapshot without
        // refreshing OAuth while the picker constructs its first frame.
        let data = AuthStorageData(
            serde_json::json!({
                "openai-codex": {
                    "type": "oauth", "access": codex_credentials.access_token,
                    "accountId": codex_credentials.account_id, "expires": 1,
                },
                "google": {
                    "type": "api_key", "key": google_credentials.api_key,
                }
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let auth = AuthStorage::in_memory_without_env(&data, Arc::new(NoOAuth));
        let mut registry = ModelRegistry::create(auth, &models_path);
        assert_eq!(registry.get_error(), None);
        let codex_models = registry
            .get_available()
            .into_iter()
            .filter(|model| model.provider == OPENAI_CODEX_PROVIDER_ID)
            .map(|model| (model.id.as_str(), model.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            codex_models,
            vec![
                ("account-only", "My Codex model"),
                ("explicit-custom", "explicit-custom"),
            ]
        );
        let google_models = registry
            .get_available()
            .into_iter()
            .filter(|model| model.provider == GOOGLE_PROVIDER_ID)
            .map(|model| (model.id.as_str(), model.name.as_str(), model.reasoning))
            .collect::<Vec<_>>();
        assert_eq!(
            google_models,
            vec![("gemini-2.5-custom-exp", "Gemini 2.5 Custom Exp", true)]
        );
        assert_eq!(
            registry
                .resolve_set_model_selection(OPENAI_CODEX_PROVIDER_ID, "account-only")
                .map(|model| model.id.as_str()),
            Ok("account-only")
        );
        registry.auth.set(
            OPENAI_CODEX_PROVIDER_ID,
            serde_json::from_value(serde_json::json!({
                "type": "oauth", "access": "account-b-token",
                "accountId": "account-b", "expires": 4_102_444_800_000i64,
            }))
            .unwrap(),
        );
        registry.auth.set(
            GOOGLE_PROVIDER_ID,
            serde_json::from_value(serde_json::json!({
                "type": "api_key", "key": "google-key-b",
            }))
            .unwrap(),
        );
        registry.refresh();
        assert!(!registry
            .get_all()
            .iter()
            .any(|model| model.id == "account-only" || model.id == "gemini-2.5-custom-exp"));
    }
}
