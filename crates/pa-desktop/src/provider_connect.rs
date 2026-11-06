//! Ephemeral UI adapter shared by provider-specific login flows.
pub(super) mod prime;
pub(super) mod subscription;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::oneshot;
use url::Url;

pub(crate) use crate::state::provider_credentials::LoginCredential as PendingCredential;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectMethod {
    Browser,
    ApiKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectStatus {
    Choose,
    Working,
    Saving,
    Failed,
}

pub(crate) struct BrowserStep {
    pub url: String,
    pub instructions: Option<String>,
    pub code: Option<String>,
}

pub(crate) struct InputPrompt {
    pub label: String,
    pub placeholder: Option<String>,
    pub secret: bool,
    pub allow_empty: bool,
}

pub(crate) struct ChoicePrompt {
    pub label: String,
    pub options: Vec<String>,
    pub selected: usize,
}

pub(crate) struct ConnectionPane {
    pub provider_id: String,
    pub display_name: String,
    pub can_browser: bool,
    pub can_api_key: bool,
    pub status: ConnectStatus,
    pub progress: String,
    pub error: Option<String>,
    pub browser: Option<BrowserStep>,
    pub prompt: Option<InputPrompt>,
    pub choice: Option<ChoicePrompt>,
}

impl ConnectionPane {
    pub fn clear_step(&mut self) {
        self.browser = None;
        self.prompt = None;
        self.choice = None;
    }
}

// Cancellation and persistence compete at the credential slot's locked commit
// point. Once committing, the UI waits for the write result before it dismisses.
#[derive(Default)]
pub(crate) struct LoginGate(AtomicU8);
impl LoginGate {
    pub(crate) fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
    pub(crate) fn begin_commit(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub(crate) fn is_committing(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }
}

pub(crate) enum LoginEvent {
    Progress(String),
    Browser(BrowserStep),
    Prompt(InputPrompt, oneshot::Sender<String>),
    Choice(ChoicePrompt, oneshot::Sender<usize>),
    InvalidBrowser,
}

#[derive(Clone)]
pub(crate) struct ConnectionUi {
    provider_id: Arc<str>,
    agent_dir: Arc<Path>,
    slot: Option<Arc<crate::state::provider_credentials::CredentialSlot>>,
    gate: Arc<LoginGate>,
    post: Arc<dyn Fn(LoginEvent) + Send + Sync>,
}

impl ConnectionUi {
    pub(crate) fn new(
        provider_id: String,
        agent_dir: PathBuf,
        slot: Option<Arc<crate::state::provider_credentials::CredentialSlot>>,
        gate: Arc<LoginGate>,
        post: impl Fn(LoginEvent) + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            agent_dir: agent_dir.into(),
            slot,
            gate,
            post: Arc::new(post),
        }
    }
    pub(crate) fn provider_id(&self) -> &str {
        &self.provider_id
    }
    pub(crate) fn agent_dir(&self) -> &Path {
        &self.agent_dir
    }
    pub(crate) fn prime_team_id_for(&self, api_key: &str) -> Option<&str> {
        self.slot.as_ref()?.prime_team_id_for(api_key)
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.gate.is_cancelled()
    }
    pub(crate) fn progress(&self, message: &str) {
        if !self.is_cancelled() {
            (self.post)(LoginEvent::Progress(message.to_owned()));
        }
    }
    pub(crate) fn browser(&self, url: &str, instructions: Option<&str>) -> bool {
        let host = match self.provider_id() {
            "prime-inference" => "app.primeintellect.ai",
            "anthropic" => "claude.ai",
            "openai-codex" => "auth.openai.com",
            "github-copilot" => "github.com",
            "xai" => "auth.x.ai",
            _ => return self.invalid_browser(),
        };
        self.browser_on_host(url, host, instructions)
    }
    pub(crate) fn browser_on_host(
        &self,
        url: &str,
        host: &str,
        instructions: Option<&str>,
    ) -> bool {
        self.present_browser(url, host, None, instructions)
    }
    pub(crate) fn browser_with_code(
        &self,
        url: &str,
        host: &str,
        code: &str,
        instructions: Option<&str>,
    ) -> bool {
        if code.is_empty()
            || code.len() > 64
            || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return self.invalid_browser();
        }
        self.present_browser(url, host, Some(code.to_owned()), instructions)
    }
    fn invalid_browser(&self) -> bool {
        if self.gate.cancel() {
            (self.post)(LoginEvent::InvalidBrowser);
        }
        false
    }
    fn present_browser(
        &self,
        raw: &str,
        expected: &str,
        code: Option<String>,
        instructions: Option<&str>,
    ) -> bool {
        if self.is_cancelled() {
            return false;
        }
        let expected = if expected.starts_with("https://") {
            expected.to_owned()
        } else {
            format!("https://{expected}")
        };
        let (Ok(url), Ok(base)) = (Url::parse(raw), Url::parse(&expected)) else {
            return self.invalid_browser();
        };
        let expected_issuer = match self.provider_id() {
            "openai-codex" => base.host_str() == Some("auth.openai.com") && base.port().is_none(),
            "anthropic" => base.host_str() == Some("claude.ai") && base.port().is_none(),
            "xai" => base.host_str() == Some("auth.x.ai") && base.port().is_none(),
            // Only the Prime config and the explicit Copilot Enterprise domain
            // prompt can designate a different origin.
            "prime-inference" | "github-copilot" => true,
            _ => false,
        };
        let allowed = expected_issuer
            && base.username().is_empty()
            && base.password().is_none()
            && base.query().is_none()
            && base.fragment().is_none()
            && url.scheme() == "https"
            && url.host_str().is_some()
            && url.origin() == base.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && !raw
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\\');
        if !allowed {
            return self.invalid_browser();
        }
        (self.post)(LoginEvent::Browser(BrowserStep {
            url: url.into(),
            instructions: instructions.map(str::to_owned),
            code,
        }));
        true
    }
    pub(crate) async fn prompt(
        &self,
        label: &str,
        placeholder: Option<&str>,
        secret: bool,
        allow_empty: bool,
    ) -> Option<String> {
        if self.is_cancelled() {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        (self.post)(LoginEvent::Prompt(
            InputPrompt {
                label: label.into(),
                placeholder: placeholder.map(str::to_owned),
                secret,
                allow_empty,
            },
            tx,
        ));
        rx.await.ok()
    }
    pub(crate) async fn choose(
        &self,
        label: &str,
        options: Vec<String>,
        selected: usize,
    ) -> Option<usize> {
        if self.is_cancelled() || options.is_empty() {
            return None;
        }
        let selected = selected.min(options.len() - 1);
        let (tx, rx) = oneshot::channel();
        (self.post)(LoginEvent::Choice(
            ChoicePrompt {
                label: label.into(),
                options,
                selected,
            },
            tx,
        ));
        rx.await.ok()
    }
}

pub(crate) async fn login(
    method: ConnectMethod,
    ui: &ConnectionUi,
) -> Result<PendingCredential, &'static str> {
    match (ui.provider_id(), method) {
        ("prime-inference", _) => prime::login(method, ui).await,
        (_, ConnectMethod::Browser) => subscription::login(ui).await,
        (_, ConnectMethod::ApiKey) => {
            let key = ui
                .prompt("API key", Some("Paste a new API key"), true, false)
                .await
                .ok_or("Connection cancelled.")?;
            let key = key.trim();
            if key.is_empty() || key.starts_with('!') {
                return Err("Enter a literal API key.");
            }
            Ok(PendingCredential::Provider(
                pa_core::auth::AuthCredential::ApiKey {
                    key: key.to_owned(),
                    prime_team: None,
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_mismatch_cancels_before_any_url_or_credential_can_be_used() {
        for raw in [
            "http://auth.openai.com/oauth/authorize",
            "https://auth.openai.com.bad.test/oauth/authorize",
            "https://user@auth.openai.com/oauth/authorize",
            "https://auth.openai.com:8443/oauth/authorize",
            "https://auth.openai.com/oauth/authorize#unexpected",
        ] {
            let (sender, events) = std::sync::mpsc::channel();
            let gate = Arc::new(LoginGate::default());
            let ui = ConnectionUi::new(
                "openai-codex".into(),
                PathBuf::new(),
                None,
                Arc::clone(&gate),
                move |event| {
                    sender.send(event).unwrap();
                },
            );
            assert!(!ui.browser(raw, None));
            assert!(matches!(events.try_recv(), Ok(LoginEvent::InvalidBrowser)));
            assert!(gate.is_cancelled() && !gate.begin_commit());
            assert!(events.try_recv().is_err());
        }
    }
}
