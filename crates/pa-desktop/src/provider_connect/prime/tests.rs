use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use pa_core::auth::{DEFAULT_PRIME_API_BASE_URL, DEFAULT_PRIME_FRONTEND_URL, PrimeHttpResponse};
use rsa::pkcs8::DecodePublicKey as _;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::provider_connect::{LoginEvent, LoginGate};
use crate::state::provider_credentials::CredentialSlot;

const KEY: &str = "fixture-inference-key";
const POLL_TOKEN: &str = "fixture-private-poll-token";

struct Service {
    calls: Mutex<Vec<&'static str>>,
    encrypted: Mutex<Option<String>>,
    deny: bool,
}

impl Service {
    fn new() -> Self {
        Self {
            calls: Mutex::default(),
            encrypted: Mutex::default(),
            deny: false,
        }
    }
}

impl PrimeHttp for Service {
    fn get(
        &self,
        url: &str,
        bearer: &str,
        _: u64,
    ) -> Pin<Box<dyn Future<Output = Result<PrimeHttpResponse, String>> + Send>> {
        let (call, status, body) = if url.ends_with("/auth_challenge/status?challenge=CODE-123") {
            assert_eq!(bearer, POLL_TOKEN);
            let encrypted = self
                .encrypted
                .lock()
                .unwrap()
                .take()
                .expect("poll follows challenge");
            ("poll", 200, json!({"result":encrypted}))
        } else if url.ends_with("/user/whoami") {
            assert_eq!(bearer, KEY);
            if self.deny {
                (
                    "access",
                    403,
                    json!({"detail":format!("echoed credential: {KEY}")}),
                )
            } else {
                (
                    "access",
                    200,
                    json!({"data":{"scope":{"inference":{"write":true}}}}),
                )
            }
        } else {
            assert!(bearer == KEY && url.ends_with("/user/teams?offset=0&limit=100"));
            (
                "teams",
                200,
                json!({"data":[{"teamId":"alpha","name":"Alpha"},
                        {"teamId":"beta","name":"Beta","slug":"research","role":"member"}],"total_count":2}),
            )
        };
        self.calls.lock().unwrap().push(call);
        Box::pin(async move {
            Ok(PrimeHttpResponse {
                status,
                body: body.to_string(),
            })
        })
    }

    fn post_json<'a>(
        &'a self,
        url: &'a str,
        body: &'a str,
        bearer: Option<&'a str>,
        _: u64,
    ) -> Pin<Box<dyn Future<Output = Result<PrimeHttpResponse, String>> + Send + 'a>> {
        assert!(url.ends_with("/auth_challenge/generate") && bearer.is_none());
        self.calls.lock().unwrap().push("challenge");
        Box::pin(async move {
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            let key = rsa::RsaPublicKey::from_public_key_pem(
                body["encryptionPublicKey"].as_str().unwrap(),
            )
            .unwrap();
            let ciphertext = key
                .encrypt(
                    &mut rsa::rand_core::OsRng,
                    rsa::Oaep::new::<rsa::sha2::Sha256>(),
                    KEY.as_bytes(),
                )
                .unwrap();
            *self.encrypted.lock().unwrap() =
                Some(base64::engine::general_purpose::STANDARD.encode(ciphertext));
            Ok(PrimeHttpResponse {
                status: 200,
                body: json!({"challenge":"CODE-123","status_auth_token":POLL_TOKEN}).to_string(),
            })
        })
    }
}

fn production() -> PrimeInferenceAuthConfig {
    PrimeInferenceAuthConfig {
        base_url: DEFAULT_PRIME_API_BASE_URL.into(),
        frontend_url: DEFAULT_PRIME_FRONTEND_URL.into(),
    }
}

fn ui(
    profile: &TempDir,
    gate: Arc<LoginGate>,
    post: impl Fn(LoginEvent) + Send + Sync + 'static,
) -> ConnectionUi {
    ConnectionUi::new(
        "prime-inference".into(),
        profile.path().into(),
        Some(Arc::new(
            CredentialSlot::read(profile.path(), "prime-inference").unwrap(),
        )),
        gate,
        post,
    )
}

fn stored_team(profile: &TempDir, key: &str) -> String {
    let raw = json!({"prime-inference":{"type":"api_key","key":key,"primeTeam":{"teamId":"alpha","name":"Alpha"}}}).to_string();
    std::fs::write(profile.path().join("auth.json"), &raw).unwrap();
    raw
}

fn safe(value: &str) {
    assert!(!value.contains(KEY) && !value.contains(POLL_TOKEN));
}

#[tokio::test]
async fn browser_challenge_is_verified_then_returns_the_selected_team() {
    let service = Service::new();
    let profile = TempDir::new().unwrap();
    let previous = stored_team(&profile, KEY);
    let ui = ui(&profile, Arc::default(), |event| match event {
        LoginEvent::Browser(step) => {
            assert_eq!(
                step.url,
                "https://app.primeintellect.ai/dashboard/tokens/challenge?code=CODE-123"
            );
            assert_eq!(step.code.as_deref(), Some("CODE-123"));
            safe(&step.url);
        }
        LoginEvent::Choice(prompt, reply) => {
            assert_eq!(prompt.options, ["Personal account", "Alpha", "Beta"]);
            assert_eq!(prompt.selected, 1);
            reply.send(2).unwrap();
        }
        LoginEvent::Progress(message) => safe(&message),
        _ => panic!("unexpected login step"),
    });
    let result = connect(
        &service,
        &production(),
        &PrimeInferenceLoginOptions::new(None),
        ConnectMethod::Browser,
        &ui,
        false,
    )
    .await
    .unwrap();
    let PendingCredential::Prime {
        api_key,
        team: PrimeTeamAssignment::Team(team),
    } = result
    else {
        panic!("expected a team key")
    };
    assert_eq!(api_key, KEY);
    assert_eq!(team.team_id, "beta");
    assert_eq!(team.slug.as_deref(), Some("research"));
    assert_eq!(team.role.as_deref(), Some("member"));
    assert_eq!(
        *service.calls.lock().unwrap(),
        ["challenge", "poll", "access", "teams"]
    );
    assert_eq!(
        std::fs::read_to_string(profile.path().join("auth.json")).unwrap(),
        previous
    );
}

#[tokio::test]
async fn a_failed_browser_access_check_does_not_publish_the_api_response() {
    let profile = TempDir::new().unwrap();
    let service = Service {
        deny: true,
        ..Service::new()
    };
    let ui = ui(&profile, Arc::default(), |event| match event {
        LoginEvent::Browser(step) => safe(&step.url),
        LoginEvent::Progress(message) => safe(&message),
        _ => panic!("denied browser key must not reach account selection"),
    });
    let result = connect(
        &service,
        &production(),
        &PrimeInferenceLoginOptions::new(None),
        ConnectMethod::Browser,
        &ui,
        false,
    )
    .await;
    let Err(message) = result else {
        panic!("denied key became a pending credential")
    };
    safe(message);
    assert_eq!(
        *service.calls.lock().unwrap(),
        ["challenge", "poll", "access"]
    );
    assert!(!profile.path().join("auth.json").exists());
}

#[tokio::test]
async fn manual_key_is_checked_and_can_select_personal_account() {
    let profile = TempDir::new().unwrap();
    stored_team(&profile, "fixture-different-identity");
    let service = Service::new();
    let ui = ui(&profile, Arc::default(), |event| match event {
        LoginEvent::Prompt(prompt, reply) => {
            assert!(prompt.secret);
            reply.send(format!(" {KEY} ")).unwrap();
        }
        LoginEvent::Choice(choice, reply) => {
            assert_eq!(
                choice.selected, 0,
                "a new key starts on the personal account"
            );
            reply.send(0).unwrap();
        }
        LoginEvent::Progress(message) => safe(&message),
        _ => panic!("unexpected login step"),
    });
    let result = connect(
        &service,
        &production(),
        &PrimeInferenceLoginOptions::new(None),
        ConnectMethod::ApiKey,
        &ui,
        false,
    )
    .await
    .unwrap();
    assert!(
        matches!(result, PendingCredential::Prime { ref api_key, team:PrimeTeamAssignment::PersonalAccount } if api_key == KEY)
    );
    assert_eq!(*service.calls.lock().unwrap(), ["access", "teams"]);
}

#[tokio::test]
async fn denied_and_cancelled_logins_return_no_pending_credential() {
    let profile = TempDir::new().unwrap();
    for deny in [false, true] {
        let gate = Arc::<LoginGate>::default();
        let cancel = gate.clone();
        let ui = ui(&profile, gate, move |event| match event {
            LoginEvent::Prompt(_, reply) => {
                reply.send(KEY.into()).unwrap();
            }
            LoginEvent::Choice(_, reply) => {
                assert!(!deny);
                cancel.cancel();
                drop(reply);
            }
            LoginEvent::Progress(message) => safe(&message),
            _ => panic!("unexpected login step"),
        });
        let service = Service {
            deny,
            ..Service::new()
        };
        let result = connect(
            &service,
            &production(),
            &PrimeInferenceLoginOptions::new(None),
            ConnectMethod::ApiKey,
            &ui,
            false,
        )
        .await;
        let Err(error) = result else {
            panic!("unexpected pending credential")
        };
        safe(error);
        if deny {
            assert_eq!(error, "This API key does not have Prime Inference access.");
            assert_eq!(*service.calls.lock().unwrap(), ["access"]);
        } else {
            assert_eq!(error, CANCELLED);
        }
    }
    assert!(!profile.path().join("auth.json").exists());
}

#[tokio::test]
async fn production_reuses_prime_cli_but_an_override_never_receives_its_key() {
    let profile = TempDir::new().unwrap();
    let cli = profile.path().join("config.json");
    for is_stock in [true, false] {
        let config = if is_stock {
            production()
        } else {
            PrimeInferenceAuthConfig {
                base_url: "https://api.fixture.test".into(),
                frontend_url: "https://browser.fixture.test/login".into(),
            }
        };
        let candidate = if is_stock {
            KEY
        } else {
            "fixture-only-for-production"
        };
        std::fs::write(
            &cli,
            json!({"api_key":candidate,"team_id":"test-team","team_name":"Research"}).to_string(),
        )
        .unwrap();
        let options = PrimeInferenceLoginOptions::new(Some(&cli));
        let ui = ui(&profile, Arc::default(), move |event| match event {
            LoginEvent::Progress(message) => safe(&message),
            LoginEvent::Browser(step) if !is_stock => {
                assert_eq!(
                    step.url,
                    "https://browser.fixture.test/login/dashboard/tokens/challenge?code=CODE-123"
                );
                safe(&step.url);
            }
            _ => panic!("unexpected sign-in interaction"),
        });
        let service = Service::new();
        let result = connect(
            &service,
            &config,
            &options,
            ConnectMethod::Browser,
            &ui,
            true,
        )
        .await
        .unwrap();
        let PendingCredential::Prime { api_key, team } = result else {
            panic!("expected a Prime key")
        };
        assert_eq!(api_key, KEY);
        if is_stock {
            assert!(matches!(team, PrimeTeamAssignment::Team(t) if t.team_id=="test-team"));
            assert_eq!(*service.calls.lock().unwrap(), ["access"]);
        } else {
            assert!(matches!(team, PrimeTeamAssignment::PreserveWhenKeyMatches));
            assert_eq!(
                *service.calls.lock().unwrap(),
                ["challenge", "poll", "access"]
            );
        }
    }
}

#[test]
fn endpoints_and_challenges_reject_extra_recipients_or_secrets() {
    let profile = TempDir::new().unwrap();
    let ui = ui(&profile, Arc::default(), |_| {
        panic!("invalid challenge was published")
    });
    for value in [
        "http://prime.example",
        "https://user:pass@prime.example",
        "https://prime.example?token=hidden",
        "https://prime.example#fragment",
    ] {
        assert!(endpoint(value).is_err());
    }
    let frontend = endpoint(DEFAULT_PRIME_FRONTEND_URL).unwrap();
    for value in [
        "https://other.example/dashboard/tokens/challenge?code=ABC",
        "https://app.primeintellect.ai/wrong-path?code=ABC",
        "https://app.primeintellect.ai/dashboard/tokens/challenge?code=A&token=hidden",
        "https://app.primeintellect.ai/dashboard/tokens/challenge?code=WRONG",
    ] {
        assert!(!show_challenge(
            &ui,
            &frontend,
            &PrimeAuthInfo {
                url: value.into(),
                instructions: "Code: ABC".into()
            }
        ));
    }
}
