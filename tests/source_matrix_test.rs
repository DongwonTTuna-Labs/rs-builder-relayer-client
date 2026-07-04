use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

const MATRIX_PATH: &str = "docs/DEPOSIT_WALLET_SOURCE_MATRIX.json";
const FIXTURE_DIR: &str = "tests/fixtures/deposit_wallet";

const REQUIRED_BEHAVIORS: &[&str] = &[
    "wallet_create_submit_body",
    "wallet_submit_body",
    "nonce_request",
    "transaction_polling",
    "deployed_wallet_check",
    "relayer_auth_header",
    "deposit_wallet_eip712_typed_data",
    "pusd_ctf_addresses",
    "adapter_route",
    "poly_1271_clob_funder_rule",
];

fn matrix() -> Value {
    let text = fs::read_to_string(MATRIX_PATH).expect("source matrix should be readable");
    serde_json::from_str(&text).expect("source matrix should be valid JSON")
}

fn required_rows(value: &Value) -> &[Value] {
    value["requiredBehaviors"]
        .as_array()
        .expect("requiredBehaviors should be an array")
}

fn row_by_id<'a>(value: &'a Value, id: &str) -> &'a Value {
    required_rows(value)
        .iter()
        .find(|row| row["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("missing source matrix row {id}"))
}

fn non_empty_string(value: &Value, key: &str) -> bool {
    value[key].as_str().is_some_and(|s| !s.trim().is_empty())
}

fn date_like(value: &Value, key: &str) -> bool {
    let Some(s) = value[key].as_str() else {
        return false;
    };
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit())
}

fn hex_of_len(value: &str, len: usize) -> bool {
    value.len() == len && value.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit())
}

#[test]
fn source_matrix_has_required_schema_and_behaviors() {
    let value = matrix();

    assert_eq!(
        value["schemaVersion"].as_str(),
        Some("paca.pbrsdk.deposit_wallet_source_matrix.v1")
    );
    assert!(date_like(&value, "retrievalDate"));

    let row_ids = required_rows(&value)
        .iter()
        .map(|row| row["id"].as_str().expect("row id should be a string"))
        .collect::<BTreeSet<_>>();

    for &required in REQUIRED_BEHAVIORS {
        assert!(row_ids.contains(required), "missing required row {required}");
        let row = row_by_id(&value, required);
        for key in [
            "id",
            "behavior",
            "wireApiSurface",
            "canonicalSource",
            "mismatchStatus",
            "fallbackDecision",
            "liveChangeGateStatus",
        ] {
            assert!(non_empty_string(row, key), "row {required} missing {key}");
        }
        assert!(date_like(row, "retrievalDate"), "row {required} missing retrievalDate");
        assert!(
            row["sourceUrlOrRepoPath"]
                .as_array()
                .is_some_and(|sources| !sources.is_empty()),
            "row {required} should cite at least one source"
        );
        assert!(
            row["relatedLocalPath"]
                .as_array()
                .is_some_and(|paths| !paths.is_empty()),
            "row {required} should cite at least one local path"
        );
    }
}

#[test]
fn source_matrix_pins_official_sdk_sources_to_exact_commits() {
    let value = matrix();
    let sdk_pins = value["officialSourcePins"]["sdk"]
        .as_array()
        .expect("official SDK pins should be an array");

    let expected = [
        (
            "https://github.com/Polymarket/builder-relayer-client",
            "9122f6fb1856f1ecfe4406685bfa19a2c5a7b290",
            "0.0.10",
        ),
        (
            "https://github.com/Polymarket/py-builder-relayer-client",
            "267a36d84d7839b6e4ac134297d9230fc224cf8f",
            "0.0.2",
        ),
        (
            "https://github.com/Polymarket/rs-clob-client-v2",
            "3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb",
            "0.6.0",
        ),
    ];

    for (repo, commit, version) in expected {
        let pin = sdk_pins
            .iter()
            .find(|pin| pin["repo"].as_str() == Some(repo))
            .unwrap_or_else(|| panic!("missing SDK pin for {repo}"));
        assert_eq!(pin["commit"].as_str(), Some(commit));
        assert_eq!(pin["packageVersion"].as_str(), Some(version));
        assert!(hex_of_len(commit, 40));
        let paths = pin["paths"].as_array().expect("SDK paths should be listed");
        assert!(!paths.is_empty(), "SDK pin {repo} should list source paths");
        for path in paths {
            let url = path["url"].as_str().expect("SDK path URL should be a string");
            assert!(url.contains(commit), "SDK path URL should contain pinned commit: {url}");
            assert!(!url.contains("/main/"), "SDK path URL should not point at main: {url}");
            assert!(!url.contains("/master/"), "SDK path URL should not point at master: {url}");
        }
    }
}

#[test]
fn wallet_nonce_discrepancy_is_explicitly_resolved() {
    let value = matrix();
    let row = row_by_id(&value, "nonce_request");
    let mismatch = row["mismatchStatus"].as_str().expect("nonce mismatch should be recorded");
    let fallback = row["fallbackDecision"].as_str().expect("nonce fallback should be recorded");
    let live_gate = row["liveChangeGateStatus"].as_str().expect("nonce gate should be recorded");

    assert!(mismatch.contains("PROXY/SAFE"), "nonce mismatch should mention public docs enum");
    assert!(mismatch.contains("WALLET"), "nonce mismatch should mention WALLET");
    assert!(
        fallback.contains("official_sdk_commit_authoritative"),
        "nonce fallback should select official SDK evidence"
    );
    assert!(
        live_gate.contains("gated") || live_gate.contains("disabled"),
        "nonce row should not silently enable live behavior"
    );
}

#[test]
fn fixture_provenance_covers_every_deposit_wallet_json_fixture() {
    let value = matrix();
    let entries = value["fixtureProvenance"]
        .as_array()
        .expect("fixtureProvenance should be an array");

    let actual = fs::read_dir(FIXTURE_DIR)
        .expect("fixture directory should exist")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<BTreeSet<_>>();

    let documented = entries
        .iter()
        .map(|entry| {
            entry["path"]
                .as_str()
                .expect("fixture path should be a string")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(documented, actual, "fixtureProvenance should exactly match fixture files");

    for entry in entries {
        let path = entry["path"].as_str().expect("fixture path should be a string");
        assert!(Path::new(path).exists(), "fixture path should exist: {path}");
        assert_eq!(entry["exists"].as_bool(), Some(true));
        assert!(hex_of_len(entry["sha256"].as_str().expect("sha256 should be a string"), 64));
        assert!(date_like(entry, "retrievalOrGenerationDate"));
        assert!(non_empty_string(entry, "sanitizerStatus"));
        assert!(non_empty_string(entry, "authorityStatus"));
        assert!(non_empty_string(entry, "decision"));
        assert!(non_empty_string(entry, "notes"));
    }
}

#[test]
fn source_matrix_and_fixture_readme_avoid_secret_values() {
    let mut text = fs::read_to_string(MATRIX_PATH).expect("source matrix should be readable");
    text.push_str(&fs::read_to_string("tests/fixtures/deposit_wallet/README.md").unwrap());

    for forbidden in [
        "BEGIN PRIVATE KEY",
        "BEGIN RSA PRIVATE KEY",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "PRIVATE_KEY=",
        "SEED_PHRASE=",
        "RELAYER_API_KEY\": \"",
        "POLY_BUILDER_API_KEY\": \"",
        "CLOB_API_KEY\": \"",
    ] {
        assert!(!text.contains(forbidden), "changed provenance docs contain forbidden secret-looking pattern {forbidden}");
    }

    assert!(
        value_has_no_live_replay_claims(&matrix()),
        "source matrix should keep live mutation blocked or out of scope"
    );
}

fn value_has_no_live_replay_claims(value: &Value) -> bool {
    required_rows(value).iter().all(|row| {
        let gate = row["liveChangeGateStatus"].as_str().unwrap_or_default();
        !matches!(gate, "enabled" | "enabled_live" | "live_ready")
    })
}
