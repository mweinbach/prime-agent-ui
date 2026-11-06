use super::*;
use serde_json::json;
use std::fs;

fn api_key(value: &str) -> LoginCredential {
    LoginCredential::Provider(AuthCredential::ApiKey {
        key: value.into(),
        prime_team: None,
    })
}

fn prime(key: &str, team: PrimeTeamAssignment) -> LoginCredential {
    LoginCredential::Prime {
        api_key: key.into(),
        team,
    }
}

#[test]
fn a_competing_refresh_of_the_same_provider_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let original = json!({"openai-codex": {"type":"oauth", "access":"fixture-access", "refresh":"one", "expires":124}});
    fs::write(&path, original.to_string()).unwrap();
    let slot = CredentialSlot::read(dir.path(), "openai-codex").unwrap();
    let mut edited = original.clone();
    edited["openai-codex"]["refresh"] = json!("other-process-refresh");
    fs::write(&path, edited.to_string()).unwrap();

    let replacement = serde_json::from_value(original["openai-codex"].clone()).unwrap();
    assert!(
        slot.save_checked(LoginCredential::Provider(replacement), || panic!(
            "a stale slot cannot claim the commit"
        ))
        .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), edited.to_string());
}

#[test]
fn login_merges_other_process_changes_and_a_cancelled_save_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let first = CredentialSlot::read(dir.path(), "openai").unwrap();
    let second = CredentialSlot::read(dir.path(), "deepseek").unwrap();
    assert!(!path.exists());
    let mut expected =
        json!({"other": {"type":"future", "opaque":[1,2]}, "mcp:test": {"type":"opaque"}});
    fs::write(&path, expected.to_string()).unwrap();
    assert!(first.save_checked(api_key("cancelled"), || false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), expected.to_string());
    first
        .save_checked(api_key("openai-fixture"), || true)
        .unwrap();
    second
        .save_checked(api_key("deepseek-fixture"), || true)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    expected["openai"] = json!({"type":"api_key","key":"openai-fixture"});
    expected["deepseek"] = json!({"type":"api_key","key":"deepseek-fixture"});
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn unknown_target_command_key_and_invalid_documents_remain_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    for document in [
        "",
        "null",
        "[]",
        "{incomplete",
        r#"{"openai":{"type":"future","opaque":1}}"#,
        r#"{"openai":{"type":"api_key","key":"!command"}}"#,
    ] {
        fs::write(&path, document).unwrap();
        assert!(CredentialSlot::read(dir.path(), "openai").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), document);
    }
    fs::write(&path, "{}").unwrap();
    let slot = CredentialSlot::read(dir.path(), "openai").unwrap();
    assert!(
        slot.save_checked(api_key(" !shell-expression"), || true)
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
    fs::write(&path, "{broken after login began").unwrap();
    assert!(slot.save_checked(api_key("fixture"), || true).is_err());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{broken after login began"
    );
}

#[test]
fn prime_team_follows_the_selected_identity_and_stays_bound_to_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let original_team = json!({"teamId":"team-a", "name":"A", "futureAttribute":true});
    fs::write(&path, json!({"prime-inference":{"type":"api_key","key":"old","primeTeam":original_team,"futureAttribute":5}}).to_string()).unwrap();
    let slot = || CredentialSlot::read(dir.path(), PRIME_INFERENCE_PROVIDER_ID).unwrap();
    let read = || serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap();
    let old = slot();
    assert_eq!(old.prime_team_id_for("old"), Some("team-a"));
    assert_eq!(old.prime_team_id_for("new"), None);
    old.save_checked(
        prime("old", PrimeTeamAssignment::PreserveWhenKeyMatches),
        || true,
    )
    .unwrap();
    assert_eq!(read()["prime-inference"]["primeTeam"], original_team);
    assert_eq!(read()["prime-inference"]["futureAttribute"], 5);
    slot()
        .save_checked(
            prime("new", PrimeTeamAssignment::PreserveWhenKeyMatches),
            || true,
        )
        .unwrap();
    assert_eq!(
        read()["prime-inference"],
        json!({"type":"api_key","key":"new","primeTeam":null})
    );
    let team = serde_json::from_value(json!({"teamId":"team-b","name":"B"})).unwrap();
    slot()
        .save_checked(prime("new", PrimeTeamAssignment::Team(team)), || true)
        .unwrap();
    assert_eq!(slot().prime_team_id_for("new"), Some("team-b"));
    slot()
        .save_checked(prime("new", PrimeTeamAssignment::PersonalAccount), || true)
        .unwrap();
    assert!(read()["prime-inference"]["primeTeam"].is_null());
}
