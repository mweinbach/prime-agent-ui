//! Adapt the pinned CLI's four subscription flows to the desktop interaction channel.
//! The harness owns PKCE, callback listeners, device polling, and token exchange.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use pa_ai::oauth::{
    self, CodexHttp, CodexLoginUi, OAuthLoginUi, OAuthPrompt, ProviderHttp, ReqwestCodexHttp,
    ReqwestProviderHttp,
};
use pa_core::auth::{
    ANTHROPIC_PROVIDER_ID, AuthCredential, GITHUB_COPILOT_PROVIDER_ID, OPENAI_CODEX_PROVIDER_ID,
    XAI_PROVIDER_ID,
};
use url::Url;

use super::{ConnectionUi, PendingCredential};

const PASTE_LABEL: &str = "Paste the authorization code or redirect URL";
const FAILED: &str = "Sign-in did not finish. Start a new sign-in and try again.";
const INVALID_BROWSER: &str = "The provider returned an invalid sign-in address or device code.";
const CANCELLED: &str = "Sign-in cancelled.";
const CALLBACK_UNAVAILABLE: &str = "Anthropic sign-in requires the local callback port 53692. Check that the port is available and try again.";

type Reply<'a> = Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;

struct LoginAdapter<'a> {
    ui: &'a ConnectionUi,
    // Copilot asks for the enterprise before returning its device authorization URL.
    github_host: Mutex<String>,
    rejected: AtomicBool,
}

impl LoginAdapter<'_> {
    fn cancelled(&self) -> bool {
        self.ui.is_cancelled() || self.rejected.load(Ordering::Acquire)
    }

    fn present(&self, url: &str, instructions: Option<&str>) {
        if self.cancelled() {
            return;
        }
        let opened = match self.ui.provider_id() {
            GITHUB_COPILOT_PROVIDER_ID | XAI_PROVIDER_ID => {
                device_code(instructions).is_some_and(|code| {
                    let host = if self.ui.provider_id() == GITHUB_COPILOT_PROVIDER_ID {
                        self.github_host
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                    } else {
                        "auth.x.ai".into()
                    };
                    self.ui.browser_with_code(
                        url,
                        &host,
                        code,
                        Some("Enter this code in the provider’s sign-in page."),
                    )
                })
            }
            _ => self.ui.browser(
                url,
                Some("Complete sign-in in your browser, or paste the redirect URL below."),
            ),
        };
        if !opened {
            self.rejected.store(true, Ordering::Release);
        }
    }

    /// Both redirect-based flows race this future against their private loopback listener.
    fn paste(&self) -> Reply<'static> {
        let ui = self.ui.clone();
        Box::pin(async move { ui.prompt(PASTE_LABEL, None, true, false).await })
    }
}

impl CodexLoginUi for LoginAdapter<'_> {
    fn on_auth(&self, url: &str, instructions: &str) {
        self.present(url, Some(instructions));
    }
    fn on_manual_code_input(&self) -> Option<Reply<'static>> {
        Some(self.paste())
    }
    fn on_prompt(&self, _: &str) -> Reply<'static> {
        self.paste()
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled()
    }
}

impl OAuthLoginUi for LoginAdapter<'_> {
    fn on_auth(&self, url: &str, instructions: Option<&str>) {
        self.present(url, instructions);
    }
    fn on_manual_code_input(&self) -> Option<Reply<'_>> {
        Some(self.paste())
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled()
    }

    fn on_prompt(&self, _: &OAuthPrompt) -> Reply<'_> {
        if self.ui.provider_id() != GITHUB_COPILOT_PROVIDER_ID {
            return self.paste();
        }
        Box::pin(async move {
            loop {
                let answer = self
                    .ui
                    .prompt(
                        "GitHub Enterprise domain (optional)",
                        Some("Leave blank for github.com"),
                        false,
                        true,
                    )
                    .await?;
                if self.cancelled() {
                    return None;
                }
                if let Some(host) = enterprise_host(&answer) {
                    self.github_host
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone_from(&host);
                    return Some(if host == "github.com" {
                        String::new()
                    } else {
                        host
                    });
                }
                self.ui.progress(
                    "Enter a GitHub Enterprise hostname or HTTPS address without a path.",
                );
            }
        })
    }

    fn on_progress(&self, _: &str) {
        // The transport may embed response bodies in errors. Keep UI narration local.
        if !self.cancelled() {
            self.ui
                .progress(if self.ui.provider_id() == GITHUB_COPILOT_PROVIDER_ID {
                    "Enabling Copilot models…"
                } else {
                    "Finishing sign-in…"
                });
        }
    }
}

/// Pin the returned verification page to the exact host the user chose. The CLI
/// forms its API endpoints from a bare domain and cannot honor a custom port or path.
fn enterprise_host(input: &str) -> Option<String> {
    let value = input.trim();
    if value.is_empty() {
        return Some("github.com".into());
    }
    if value
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let url = if value.contains("://") {
        Url::parse(value)
    } else {
        Url::parse(&format!("https://{value}"))
    }
    .ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.domain()
        .map(|host| host.trim_end_matches('.'))
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
}

fn device_code(instructions: Option<&str>) -> Option<&str> {
    let code = instructions?.strip_prefix("Enter code: ")?;
    (!code.is_empty()
        && code.len() <= 32
        && code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    .then_some(code)
}

pub(super) async fn login(ui: &ConnectionUi) -> Result<PendingCredential, &'static str> {
    login_with_transports(ui, &ReqwestCodexHttp::new(), &ReqwestProviderHttp::new()).await
}

async fn login_with_transports(
    ui: &ConnectionUi,
    codex: &dyn CodexHttp,
    provider: &dyn ProviderHttp,
) -> Result<PendingCredential, &'static str> {
    let adapter = LoginAdapter {
        ui,
        github_host: Mutex::new("github.com".into()),
        rejected: AtomicBool::new(false),
    };
    if adapter.cancelled() {
        return Err(CANCELLED);
    }
    let result = match ui.provider_id() {
        OPENAI_CODEX_PROVIDER_ID => {
            oauth::login_openai_codex(codex, &adapter, oauth::DEFAULT_ORIGINATOR)
                .await
                .map(|c| credential(c.access, c.refresh, c.expires, Some(c.account_id), None))
        }
        ANTHROPIC_PROVIDER_ID => oauth::login_anthropic(provider, &adapter)
            .await
            .map(|c| credential(c.access, c.refresh, c.expires, None, None)),
        GITHUB_COPILOT_PROVIDER_ID => oauth::login_github_copilot(provider, &adapter)
            .await
            .map(|c| credential(c.access, c.refresh, c.expires, None, c.enterprise_url)),
        XAI_PROVIDER_ID => oauth::login_xai(provider, &adapter)
            .await
            .map(|c| credential(c.access, c.refresh, c.expires, None, None)),
        _ => return Err("This provider does not support subscription sign-in."),
    };
    if adapter.rejected.load(Ordering::Acquire) {
        return Err(INVALID_BROWSER);
    }
    if ui.is_cancelled() {
        return Err(CANCELLED);
    }
    // Return a candidate only. The common login gate decides whether it may be saved.
    result.map(PendingCredential::Provider).map_err(|message| {
        if message == oauth::LOGIN_CANCELLED {
            CANCELLED
        } else if ui.provider_id() == ANTHROPIC_PROVIDER_ID && message.starts_with("port 53692:") {
            CALLBACK_UNAVAILABLE
        } else {
            FAILED
        }
    })
}

fn credential(
    access: String,
    refresh: String,
    expires: i64,
    account_id: Option<String>,
    enterprise_url: Option<String>,
) -> AuthCredential {
    AuthCredential::Oauth {
        access,
        refresh: Some(refresh),
        expires,
        account_id,
        enterprise_url,
        endpoint: None,
        token_endpoint: None,
        client_id: None,
        resource: None,
        issuer: None,
        audience_mode: None,
    }
}

#[cfg(test)]
mod tests;
