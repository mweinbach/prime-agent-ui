//! Prime Inference browser challenge, key validation, and account selection.
//! Credentials leave this worker only as a pending, guarded store update.

use futures::future::{AbortHandle, Abortable};
use pa_core::auth::{
    DEFAULT_REQUEST_TIMEOUT_MS, PrimeAccessError, PrimeAuthInfo, PrimeHttp,
    PrimeInferenceAuthConfig, PrimeInferenceLoginCallbacks, PrimeInferenceLoginOptions,
    PrimeTeamAssignment, ReqwestPrimeHttp, check_prime_inference_access,
    default_prime_cli_config_path, fetch_prime_teams, login_prime_inference,
    resolve_prime_inference_auth_config,
};
use url::Url;

use super::{ConnectMethod, ConnectionUi, PendingCredential};

const CANCELLED: &str = "Prime sign-in was cancelled.";
const INVALID_LINK: &str = "Prime returned an invalid browser sign-in link.";

pub(super) async fn login(
    method: ConnectMethod,
    ui: &ConnectionUi,
) -> Result<PendingCredential, &'static str> {
    let config = resolve_prime_inference_auth_config();
    // Like the CLI, never look up the global Prime CLI credential while opening a
    // different profile. The core also disables reuse unless both hosts are stock.
    let candidate = (config.is_production()
        && pa_types::platform::agent_dir().as_deref() == Some(ui.agent_dir()))
    .then(default_prime_cli_config_path);
    let options = PrimeInferenceLoginOptions::new(candidate.as_deref());
    let team_pinned = std::env::var("PRIME_TEAM_ID").is_ok_and(|id| !id.trim().is_empty());
    connect(
        &ReqwestPrimeHttp,
        &config,
        &options,
        method,
        ui,
        team_pinned,
    )
    .await
}

fn endpoint(base: &str) -> Result<Url, &'static str> {
    let url = Url::parse(base).map_err(|_| "The configured Prime endpoint is invalid.")?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Prime sign-in requires an HTTPS endpoint without embedded credentials, a query, or a fragment.",
        );
    }
    Ok(url)
}

/// The core constructs this link from the selected frontend and the server's
/// challenge. Only the intended short-lived user code may enter desktop state.
fn show_challenge(ui: &ConnectionUi, frontend: &Url, info: &PrimeAuthInfo) -> bool {
    let Some(code) = info.instructions.strip_prefix("Code: ") else {
        return false;
    };
    let Ok(link) = Url::parse(&info.url) else {
        return false;
    };
    let path = format!(
        "{}/dashboard/tokens/challenge",
        frontend.path().trim_end_matches('/')
    );
    let mut query = link.query_pairs();
    if link.origin() != frontend.origin()
        || link.path() != path
        || !link.username().is_empty()
        || link.password().is_some()
        || link.fragment().is_some()
        || !query
            .next()
            .is_some_and(|(key, value)| key == "code" && value == code)
        || query.next().is_some()
    {
        return false;
    }
    ui.browser_with_code(link.as_str(), frontend.as_str(), code, None)
}

async fn connect(
    http: &dyn PrimeHttp,
    config: &PrimeInferenceAuthConfig,
    options: &PrimeInferenceLoginOptions<'_>,
    method: ConnectMethod,
    ui: &ConnectionUi,
    team_pinned: bool,
) -> Result<PendingCredential, &'static str> {
    let api = endpoint(&config.base_url)?;
    let frontend = endpoint(&config.frontend_url)?;
    if ui.is_cancelled() {
        return Err(CANCELLED);
    }
    let (api_key, initial_team) = match method {
        ConnectMethod::Browser => {
            ui.progress("Connecting to Prime…");
            let (reject, registration) = AbortHandle::new_pair();
            let on_auth = |info: &PrimeAuthInfo| {
                if !show_challenge(ui, &frontend, info) {
                    reject.abort();
                }
            };
            // Raw core progress and HTTP errors can contain response bodies. They do
            // not cross into desktop state or logs.
            let callbacks = PrimeInferenceLoginCallbacks {
                on_auth: &on_auth,
                on_progress: None,
            };
            let login = login_prime_inference(http, config, options, &callbacks);
            let result = Abortable::new(login, registration).await;
            if ui.is_cancelled() {
                return Err(CANCELLED);
            }
            if reject.is_aborted() {
                return Err(INVALID_LINK);
            }
            let result = result.map_err(|_| INVALID_LINK)?.map_err(
                |_| "Prime browser sign-in failed. Try again or connect with an API key.",
            )?;
            (result.api_key, result.prime_team)
        }
        ConnectMethod::ApiKey => {
            let label = if config.is_production() {
                "Prime API key".to_owned()
            } else {
                format!("Prime API key for {}", api.origin().ascii_serialization())
            };
            let input = ui
                .prompt(&label, Some("Paste your API key"), true, false)
                .await
                .ok_or(CANCELLED)?;
            let key = input.trim();
            if key.is_empty() || key.starts_with('!') || key.chars().any(char::is_control) {
                return Err("Enter a valid Prime API key.");
            }
            if ui.is_cancelled() {
                return Err(CANCELLED);
            }
            ui.progress("Checking Prime Inference access…");
            check_prime_inference_access(http, &config.base_url, key, DEFAULT_REQUEST_TIMEOUT_MS)
                .await
                .map_err(|err| match err {
                    PrimeAccessError::Denied(_) => {
                        "This API key does not have Prime Inference access."
                    }
                    PrimeAccessError::Failed(_) => {
                        "Could not verify Prime Inference access. Try again."
                    }
                })?;
            (key.to_owned(), PrimeTeamAssignment::PreserveWhenKeyMatches)
        }
    };
    if ui.is_cancelled() {
        return Err(CANCELLED);
    }
    if api_key.is_empty() || api_key.starts_with('!') || api_key.chars().any(char::is_control) {
        return Err("Prime did not return a usable API key.");
    }
    let team = if team_pinned {
        initial_team
    } else {
        ui.progress("Loading Prime accounts…");
        let teams = fetch_prime_teams(http, &config.base_url, &api_key, DEFAULT_REQUEST_TIMEOUT_MS)
            .await
            .map_err(|_| "Could not load Prime accounts. Try connecting again.")?;
        if ui.is_cancelled() {
            return Err(CANCELLED);
        }
        if teams.is_empty() {
            PrimeTeamAssignment::PersonalAccount
        } else {
            let current = match &initial_team {
                PrimeTeamAssignment::Team(team) => Some(team.team_id.as_str()),
                PrimeTeamAssignment::PersonalAccount => None,
                PrimeTeamAssignment::PreserveWhenKeyMatches => ui.prime_team_id_for(&api_key),
            };
            let selected = teams
                .iter()
                .position(|team| Some(team.team_id.as_str()) == current)
                .map_or(0, |i| i + 1);
            let names = std::iter::once("Personal account".to_owned())
                .chain(teams.iter().map(|team| {
                    // Treat returned account names as labels, never as markup or new lines.
                    team.name
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(120)
                        .collect()
                }))
                .collect();
            let choice = ui
                .choose("Choose your Prime account", names, selected)
                .await
                .ok_or(CANCELLED)?;
            if choice == 0 {
                PrimeTeamAssignment::PersonalAccount
            } else {
                PrimeTeamAssignment::Team(
                    teams
                        .into_iter()
                        .nth(choice - 1)
                        .ok_or("Unknown Prime account selection.")?,
                )
            }
        }
    };
    if ui.is_cancelled() {
        return Err(CANCELLED);
    }
    Ok(PendingCredential::Prime { api_key, team })
}

#[cfg(test)]
mod tests;
