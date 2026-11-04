mod common;

use pa_models::openai_codex::{CodexCatalog, CodexCredentials};
use pa_models::Model;
use pa_types::ai::{ModelInput, ModelThinkingLevel, EXTENDED_THINKING_LEVELS};
use serde_json::{json, Value};

fn credentials(account_id: &str, access_token: &str) -> CodexCredentials {
    CodexCredentials {
        access_token: access_token.into(),
        account_id: account_id.into(),
    }
}

fn wire_model(slug: &str) -> Value {
    json!({
        "slug": slug,
        "display_name": "Authorized model",
        "visibility": "list",
        "supported_in_api": false,
        "context_window": 400_000,
        "input_modalities": ["text"],
        "supported_reasoning_levels": [{"effort": "high"}, {"effort": "xhigh"}, {"effort": "future"}],
        "baseUrl": "https://attacker.invalid",
        "provider": "attacker",
        "api": "openai-completions",
        "headers": {"Authorization": "attacker"},
    })
}

fn expected_model(id: &str) -> Model {
    Model {
        id: id.into(),
        name: "Authorized model".into(),
        provider: "openai-codex".into(),
        api: "openai-codex-responses".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        input: vec![ModelInput::Text],
        reasoning: true,
        thinking_level_map: Some(
            EXTENDED_THINKING_LEVELS
                .into_iter()
                .map(|level| {
                    let effort = match level {
                        ModelThinkingLevel::High => Some("high".into()),
                        ModelThinkingLevel::Xhigh => Some("xhigh".into()),
                        ModelThinkingLevel::Off
                        | ModelThinkingLevel::Minimal
                        | ModelThinkingLevel::Low
                        | ModelThinkingLevel::Medium
                        | ModelThinkingLevel::Max => None,
                    };
                    (level, effort)
                })
                .collect(),
        ),
        cost: pa_ai::types::zero_model_cost(),
        context_window: 400_000,
        max_tokens: 128_000,
        max_tokens_explicit: false,
        featured: None,
        headers: None,
        compat: None,
    }
}

#[tokio::test]
async fn authorized_models_are_pinned_and_cached_by_account_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut hidden = wire_model("hidden-model");
    hidden["visibility"] = json!("hide");
    let mut unavailable = wire_model("unavailable-model");
    unavailable["visibility"] = json!("none");
    let mut no_text = wire_model("audio-model");
    no_text["input_modalities"] = json!(["audio"]);
    let mut no_efforts = wire_model("codex-without-effort-controls");
    no_efforts["supported_in_api"] = json!(true);
    no_efforts
        .as_object_mut()
        .unwrap()
        .remove("supported_reasoning_levels");
    let payload = json!({"models": [
        wire_model("codex-newly-authorized"),
        no_efforts,
        hidden,
        unavailable,
        no_text,
        {"slug": "malformed"},
    ]});
    let server =
        common::MockServer::start(vec![common::ok_json(payload.to_string(), Some("v1"))]).await;
    let catalog = CodexCatalog::with_base_url(Some(dir.path().into()), &server.url("/codex"));
    let account_a = credentials("acct-secret-a", "token-secret-a");
    let account_b = credentials("acct-secret-b", "token-secret-a");
    let rotated = credentials("acct-secret-a", "rotated-token-secret-a");
    assert_eq!(catalog.get(&account_a), None);
    let mut without_controls = expected_model("codex-without-effort-controls");
    without_controls.reasoning = false;
    without_controls.thinking_level_map = Some(
        EXTENDED_THINKING_LEVELS
            .into_iter()
            .map(|level| (level, None))
            .collect(),
    );
    let expected = Some(vec![
        expected_model("codex-newly-authorized"),
        without_controls,
    ]);
    let (first, coalesced) = tokio::join!(
        catalog.refresh(&account_a, /*force*/ true),
        catalog.refresh(&account_a, /*force*/ true),
    );
    assert_eq!(first, expected);
    assert_eq!(coalesced, expected);
    assert_eq!(catalog.refresh(&account_a, /*force*/ false).await, expected);
    let requests = server.recorded_requests();
    assert_eq!(
        requests.len(),
        1,
        "coalesced and hourly-gated requests reuse the snapshot"
    );
    let request = &requests[0];
    assert!(
        request.starts_with("GET /codex/models?client_version=0.167.0 HTTP/1.1\r\n"),
        "{request}"
    );
    let expected_ua = format!("user-agent: {}\r\n", pa_ai::oauth::codex_user_agent());
    for header in [
        "authorization: Bearer token-secret-a\r\n",
        "chatgpt-account-id: acct-secret-a\r\n",
        "originator: codex_cli_rs\r\n",
        "version: 0.167.0\r\n",
        expected_ua.as_str(),
    ] {
        assert!(request.contains(header), "{request}");
    }
    assert_eq!(catalog.get(&account_b), None);
    assert_eq!(catalog.get(&rotated), None);
    let restarted = CodexCatalog::with_base_url(Some(dir.path().into()), &server.url("/codex"));
    assert_eq!(restarted.get(&account_a), expected);
    assert_eq!(restarted.get(&account_b), None);
    assert_eq!(restarted.get(&rotated), None);
    let stored: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(stored.len(), 1);
    let snapshot = std::fs::read_to_string(&stored[0]).unwrap();
    let filename = stored[0].file_name().unwrap().to_string_lossy();
    for secret in ["token-secret-a", "acct-secret-a"] {
        assert!(!snapshot.contains(secret));
        assert!(!filename.contains(secret));
    }
}

#[tokio::test]
async fn transient_failures_keep_models_and_revocation_clears_only_its_scope() {
    let dir = tempfile::tempdir().unwrap();
    let payload = |id: &str| json!({"models": [wire_model(id)]}).to_string();
    let server = common::MockServer::start(vec![
        common::ok_json(payload("codex-account-a"), None),
        common::ok_json(payload("codex-account-b"), None),
        common::status(500, "Unavailable"),
        common::ok_json("{}", None),
        common::status(401, "Unauthorized"),
        common::status(403, "Forbidden"),
        common::ok_json(r#"{"models":[]}"#, None),
    ])
    .await;
    let catalog = CodexCatalog::with_base_url(Some(dir.path().into()), &server.url("/codex"));
    let account_a = credentials("account-a", "token-a");
    let account_b = credentials("account-b", "token-b");
    let models_a = Some(vec![expected_model("codex-account-a")]);
    let models_b = Some(vec![expected_model("codex-account-b")]);
    assert_eq!(catalog.refresh(&account_a, /*force*/ true).await, models_a);
    assert_eq!(catalog.refresh(&account_b, /*force*/ true).await, models_b);
    assert_eq!(catalog.refresh(&account_a, /*force*/ true).await, models_a);
    assert_eq!(catalog.refresh(&account_a, /*force*/ true).await, models_a);
    assert_eq!(catalog.refresh(&account_a, /*force*/ true).await, None);
    let restarted = CodexCatalog::with_base_url(Some(dir.path().into()), &server.url("/codex"));
    assert_eq!(restarted.get(&account_a), None);
    assert_eq!(restarted.get(&account_b), models_b);
    assert_eq!(catalog.refresh(&account_b, /*force*/ true).await, None);
    assert_eq!(catalog.get(&account_b), None);
    assert_eq!(
        catalog.refresh(&account_a, /*force*/ true).await,
        Some(vec![])
    );
    let restarted = CodexCatalog::with_base_url(Some(dir.path().into()), &server.url("/codex"));
    assert_eq!(restarted.get(&account_a), Some(vec![]));
    assert_eq!(restarted.get(&account_b), None);
}

#[tokio::test]
async fn google_models_filter_generation_methods_and_scope_by_key() {
    use pa_models::google::{GoogleCatalog, GoogleCredentials};
    let dir = tempfile::tempdir().unwrap();
    let payload = json!({"models": [
        {
            "name": "models/gemini-2.5-custom-pro",
            "displayName": "Gemini 2.5 Custom Pro",
            "inputTokenLimit": 2_000_000,
            "outputTokenLimit": 64_000,
            "supportedGenerationMethods": ["generateContent", "countTokens"],
            "thinking": true,
            "baseUrl": "https://attacker.invalid",
            "headers": {"Authorization": "attacker"},
        },
        {
            "name": "models/text-embedding-004",
            "displayName": "Text Embedding 004",
            "supportedGenerationMethods": ["embedContent", "generateContent"],
        },
        {
            "name": "models/imagen-3.0-generate-002",
            "displayName": "Imagen 3",
            "supportedGenerationMethods": ["generateContent"],
        },
        {
            "name": "models/gemini-count-only",
            "displayName": "Count Only",
            "supportedGenerationMethods": ["countTokens"],
        },
    ]});
    let server = common::MockServer::start(vec![
        common::ok_json(payload.to_string(), None),
        common::status(403, "Forbidden"),
    ])
    .await;
    let catalog = GoogleCatalog::with_base_url(Some(dir.path().into()), &server.url("/v1beta"));
    let key_a = GoogleCredentials {
        api_key: "gemini-secret-key-a".into(),
    };
    let key_b = GoogleCredentials {
        api_key: "gemini-secret-key-b".into(),
    };
    let refreshed = catalog.refresh(&key_a, /*force*/ true).await;
    assert_eq!(
        refreshed,
        Some(vec![Model {
            id: "gemini-2.5-custom-pro".into(),
            name: "Gemini 2.5 Custom Pro".into(),
            provider: "google".into(),
            api: "google-generative-ai".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            input: vec![ModelInput::Text, ModelInput::Image],
            reasoning: true,
            thinking_level_map: None,
            cost: pa_ai::types::zero_model_cost(),
            context_window: 2_000_000,
            max_tokens: 64_000,
            max_tokens_explicit: false,
            featured: None,
            headers: None,
            compat: None,
        }])
    );
    let requests = server.recorded_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /v1beta/models?pageSize=1000 HTTP/1.1\r\n"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("x-goog-api-key: gemini-secret-key-a\r\n"),
        "{}",
        requests[0]
    );
    assert_eq!(catalog.get(&key_b), None);
    let stored: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(stored.len(), 1);
    let snapshot = std::fs::read_to_string(&stored[0]).unwrap();
    assert!(!snapshot.contains("gemini-secret-key-a"));
    assert_eq!(catalog.refresh(&key_a, /*force*/ true).await, None);
    assert_eq!(catalog.get(&key_a), None);
}
