use std::fs;
use std::path::Path;

const SOURCE_MATRIX: &str = "docs/DEPOSIT_WALLET_SOURCE_MATRIX.md";
const FIXTURE_PROVENANCE: &str = "tests/fixtures/deposit_wallet/PROVENANCE.md";
const FIXTURE_DIR: &str = "tests/fixtures/deposit_wallet";

const REQUIRED_MATRIX_COLUMNS: &[&str] = &[
    "Row ID",
    "Behavior",
    "Wire/API surface",
    "Canonical source",
    "Source URL or repository path",
    "SDK language",
    "SDK commit/version",
    "Retrieval date",
    "Related local path",
    "Mismatch status",
    "Fallback decision",
    "Live-change gate",
];

const REQUIRED_MATRIX_ROWS: &[&str] = &[
    "SM-WALLET-CREATE",
    "SM-WALLET-BATCH",
    "SM-WALLET-NONCE",
    "SM-TRANSACTION-POLLING",
    "SM-DEPLOYED-WALLET-CHECK",
    "SM-RELAYER-AUTH-HEADERS",
    "SM-EIP712-BATCH",
    "SM-PUSD-CTF-ADDRESSES",
    "SM-ADAPTER-ROUTE",
    "SM-CLOB-POLY-1271-FUNDER",
];

#[test]
fn source_matrix_has_required_rows_and_decision_fields() {
    let matrix = fs::read_to_string(SOURCE_MATRIX).expect("source matrix should be readable");

    for column in REQUIRED_MATRIX_COLUMNS {
        assert!(
            matrix.contains(column),
            "source matrix is missing required column {column:?}"
        );
    }

    for row_id in REQUIRED_MATRIX_ROWS {
        assert!(
            matrix.contains(row_id),
            "source matrix is missing required row {row_id:?}"
        );
    }
}

#[test]
fn source_matrix_documents_wallet_nonce_discrepancy() {
    let matrix = fs::read_to_string(SOURCE_MATRIX).expect("source matrix should be readable");

    assert!(
        matrix.contains("public nonce endpoint page lists `PROXY` and `SAFE`"),
        "source matrix must record the public nonce endpoint mismatch"
    );
    assert!(
        matrix.contains("official relayer SDKs call `TransactionType.WALLET`"),
        "source matrix must record the official SDK WALLET nonce evidence"
    );
    assert!(
        matrix.contains("No live change from public docs alone"),
        "source matrix must keep WALLET nonce live behavior conservatively gated"
    );
}

#[test]
fn fixture_provenance_covers_every_deposit_wallet_fixture() {
    let provenance =
        fs::read_to_string(FIXTURE_PROVENANCE).expect("fixture provenance should be readable");
    let fixture_dir = Path::new(FIXTURE_DIR);
    let entries = fs::read_dir(fixture_dir).expect("deposit wallet fixture dir should be readable");
    let mut fixture_count = 0;

    for entry in entries {
        let entry = entry.expect("fixture dir entry should be readable");
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fixture file name should be valid UTF-8");
        fixture_count += 1;

        assert!(
            provenance.contains(&format!("| `{file_name}` |")),
            "fixture provenance is missing {file_name}"
        );
    }

    assert!(fixture_count > 0, "deposit wallet fixture coverage should be non-empty");
    assert!(
        provenance.contains("No production private key, API credential, auth header, production signature, funded account, or replayable live submit body is authoritative fixture material."),
        "fixture provenance must state the non-replayable secret policy"
    );
}
