use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use pa_ai::oauth::{CodexHttpResponse, ProviderHttpRequest, ProviderHttpResponse};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::provider_connect::{LoginEvent, LoginGate};

// Every response is local. Even the negative cases cannot fall through to reqwest.
struct ScriptedHttp {
    github: &'static str,
    verification: Option<&'static str>,
    fail_exchange: bool,
    cancel_exchange: Option<Arc<LoginGate>>,
}

impl Default for ScriptedHttp {
    fn default() -> Self {
        Self {
            github: "github.com",
            verification: None,
            fail_exchange: false,
            cancel_exchange: None,
        }
    }
}

type Response<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

impl CodexHttp for ScriptedHttp {
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        body: &'a str,
        _: u64,
    ) -> Response<'a, CodexHttpResponse> {
        Box::pin(async move {
            assert_eq!(url, "https://auth.openai.com/oauth/token");
            assert!(body.contains("code=local-code"));
            let claims = URL_SAFE_NO_PAD.encode(
                br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"test-account"}}"#,
            );
            let response = json!({"access_token":format!("fixture.{claims}.test"),"refresh_token":"local-refresh","expires_in":3600});
            Ok(CodexHttpResponse {
                status: 200,
                body: response.to_string(),
            })
        })
    }
}

impl ProviderHttp for ScriptedHttp {
    fn request(&self, req: ProviderHttpRequest, _: u64) -> Response<'_, ProviderHttpResponse> {
        Box::pin(async move {
            let endpoint = Url::parse(&req.url).unwrap();
            let device = |uri: String| {
                json!({"device_code":"private-polling-code", "user_code":"ABCD-EFGH",
                "verification_uri":self.verification.map_or(uri, str::to_owned),"interval":0,"expires_in":900})
            };
            let body: Value = match (endpoint.host_str().unwrap(), endpoint.path()) {
                ("auth.x.ai", "/oauth2/device/code") => device("https://auth.x.ai/activate".into()),
                (host, "/login/device/code") if host == self.github => {
                    device(format!("https://{host}/login/device"))
                }
                ("auth.x.ai", "/oauth2/token") | ("platform.claude.com", "/v1/oauth/token") => {
                    if self.fail_exchange {
                        return Err("response includes FAKE-RESPONSE-SECRET".into());
                    }
                    if let Some(gate) = &self.cancel_exchange {
                        gate.cancel();
                    }
                    json!({"access_token":"test-access","refresh_token":"test-refresh","expires_in":3600})
                }
                (host, "/login/oauth/access_token") if host == self.github => {
                    json!({"access_token":"test-github-refresh"})
                }
                (host, "/copilot_internal/v2/token") if host == format!("api.{}", self.github) => {
                    json!({"token":"test-copilot","expires_at":4_000_000_000_u64})
                }
                (_, path) if path.starts_with("/models/") && path.ends_with("/policy") => json!({}),
                _ => panic!("Unexpected endpoint on the scripted transport"),
            };
            Ok(ProviderHttpResponse {
                status: 200,
                body: body.to_string(),
            })
        })
    }
}

async fn run(
    provider: &str,
    scripted: &ScriptedHttp,
    gate: Arc<LoginGate>,
    enterprise: &'static str,
) -> Result<PendingCredential, &'static str> {
    let dir = TempDir::new().unwrap();
    let provider_id = provider.to_owned();
    let ui = ConnectionUi::new(
        provider.into(),
        dir.path().into(),
        None,
        gate,
        move |event| match event {
            LoginEvent::Browser(step) => {
                let redirect = matches!(
                    provider_id.as_str(),
                    OPENAI_CODEX_PROVIDER_ID | ANTHROPIC_PROVIDER_ID
                );
                assert!(!step.url.contains("private-polling-code"));
                assert_eq!(step.code.as_deref(), (!redirect).then_some("ABCD-EFGH"));
            }
            LoginEvent::Prompt(prompt, reply) => {
                if provider_id == GITHUB_COPILOT_PROVIDER_ID {
                    assert!(prompt.allow_empty && !prompt.secret);
                    let _ = reply.send(enterprise.into());
                } else {
                    assert!(prompt.secret && !prompt.allow_empty);
                    let _ = reply.send("local-code".into());
                }
            }
            LoginEvent::InvalidBrowser | LoginEvent::Progress(_) => {}
            LoginEvent::Choice(_, _) => panic!("Unexpected choice in subscription login"),
        },
    );
    let result = login_with_transports(&ui, scripted, scripted).await;
    assert!(!dir.path().join("auth.json").exists());
    result
}

#[tokio::test]
async fn subscription_candidates_preserve_cli_fields_and_do_not_write() {
    for (provider, enterprise) in [
        (OPENAI_CODEX_PROVIDER_ID, ""),
        (ANTHROPIC_PROVIDER_ID, ""),
        (XAI_PROVIDER_ID, ""),
        (GITHUB_COPILOT_PROVIDER_ID, ""),
        (GITHUB_COPILOT_PROVIDER_ID, "https://github.com/"),
        (GITHUB_COPILOT_PROVIDER_ID, "https://GIT.example.test/"),
    ] {
        let http = ScriptedHttp {
            github: if enterprise.contains("example.test") {
                "git.example.test"
            } else {
                "github.com"
            },
            ..Default::default()
        };
        let result = run(provider, &http, Arc::new(LoginGate::default()), enterprise).await;
        // The Anthropic grant requires a local listener, even when its code is
        // pasted. On a network-isolated worker exercise that specific failure;
        // on normal desktops and CI, require the full scripted success path.
        if provider == ANTHROPIC_PROVIDER_ID
            && std::net::TcpListener::bind(("127.0.0.1", 0))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        {
            assert!(matches!(result, Err(CALLBACK_UNAVAILABLE)));
            continue;
        }
        let candidate =
            result.unwrap_or_else(|error| panic!("{provider} scripted login failed: {error}"));
        let PendingCredential::Provider(AuthCredential::Oauth {
            access,
            refresh,
            expires,
            account_id,
            enterprise_url,
            ..
        }) = candidate
        else {
            panic!("expected OAuth candidate");
        };
        assert!(expires > 0 && !access.is_empty() && refresh.is_some());
        assert_eq!(
            account_id.as_deref(),
            (provider == OPENAI_CODEX_PROVIDER_ID).then_some("test-account")
        );
        assert_eq!(
            enterprise_url.as_deref(),
            enterprise
                .contains("example.test")
                .then_some("git.example.test")
        );
        if provider == GITHUB_COPILOT_PROVIDER_ID {
            assert!(access == "test-copilot" && refresh.as_deref() == Some("test-github-refresh"));
        }
    }
}

#[tokio::test]
async fn cancelled_exchanges_and_untrusted_verification_pages_yield_no_credential() {
    let gate = Arc::new(LoginGate::default());
    let late = ScriptedHttp {
        cancel_exchange: Some(Arc::clone(&gate)),
        ..Default::default()
    };
    assert!(matches!(
        run(XAI_PROVIDER_ID, &late, gate, "").await,
        Err(CANCELLED)
    ));
    let untrusted = ScriptedHttp {
        verification: Some("https://redirect.example.test/activate"),
        ..Default::default()
    };
    assert!(matches!(
        run(XAI_PROVIDER_ID, &untrusted, Arc::default(), "").await,
        Err(INVALID_BROWSER)
    ));
    let failure = ScriptedHttp {
        fail_exchange: true,
        ..Default::default()
    };
    assert!(matches!(
        run(XAI_PROVIDER_ID, &failure, Arc::default(), "").await,
        Err(FAILED)
    ));
}

#[test]
fn enterprise_addresses_and_displayed_device_codes_are_bounded() {
    assert_eq!(enterprise_host(""), Some("github.com".into()));
    assert_eq!(
        enterprise_host("Git.example.test"),
        Some("git.example.test".into())
    );
    for input in [
        "https://git.example.test/extra",
        "https://user:pass@git.example.test",
        "http://git.example.test",
        "git.example.test:8443",
    ] {
        assert!(enterprise_host(input).is_none());
    }
    assert_eq!(
        device_code(Some("Enter code: ABCD-EFGH")),
        Some("ABCD-EFGH")
    );
    assert!(device_code(Some("Enter code: pretend\nother")).is_none());
    assert!(device_code(Some(&format!("Enter code: {}", "A".repeat(64)))).is_none());
}
