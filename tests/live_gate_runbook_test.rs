//! Offline, shape-limited audit for the manual live-gate runbook.
//!
//! This audit reads only `docs/MANUAL_LIVE_GATE_RUNBOOK.md` for secret-shaped
//! values. It detects `0x`/`0X`-prefixed ASCII hex runs and two literal marker
//! forms. It does not detect unprefixed 40-hex values, UUID-shaped keys, or
//! base64 values. Passing it verifies only the specified shapes, not the
//! absence of every possible secret.

use std::fs;
use std::path::Path;

const REQUIRED_SECTIONS: &[&str] = &[
    "## Scope and non-goals",
    "## Preconditions",
    "## Secret handling",
    "## Identity separation check",
    "## Approval gates",
    "## Tiny-value bounds",
    "## Stop conditions",
    "## Rollback",
    "## Redacted evidence checklist",
    "## What this runbook does not prove",
];

const REQUIRED_STOP_CONDITIONS: &[&str] = &[
    "source drift",
    "unexpected state",
    "unknown state",
    "timeout",
    "missing transactionID",
    "wrong owner/funder",
    "redaction failure",
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Violation {
    MissingSection(&'static str),
    MissingStopCondition(&'static str),
    MissingDesignReference,
    MissingApprovalGate(usize),
    FullAddress { run_length: usize },
    SecretLike { run_length: usize },
    SignatureLike { run_length: usize },
    PemMarker,
    BearerTokenLike,
}

fn audit(runbook: &str, design: &str) -> Vec<Violation> {
    let mut violations = Vec::new();

    for &heading in REQUIRED_SECTIONS {
        if !runbook.lines().any(|line| line == heading) {
            violations.push(Violation::MissingSection(heading));
        }
    }

    for &condition in REQUIRED_STOP_CONDITIONS {
        if !runbook.contains(condition) {
            violations.push(Violation::MissingStopCondition(condition));
        }
    }

    if !design.contains("MANUAL_LIVE_GATE_RUNBOOK.md") {
        violations.push(Violation::MissingDesignReference);
    }

    for n in 1..=6 {
        let gate = format!("**GATE {n} — requires operator approval**");
        if !runbook.lines().any(|line| line == gate.as_str()) {
            violations.push(Violation::MissingApprovalGate(n));
        }
    }

    audit_secret_shapes(runbook, &mut violations);
    violations
}

fn audit_secret_shapes(runbook: &str, violations: &mut Vec<Violation>) {
    let bytes = runbook.as_bytes();
    let mut index = 0;

    while index + 1 < bytes.len() {
        if bytes[index] == b'0' && matches!(bytes[index + 1], b'x' | b'X') {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            let run_length = end - (index + 2);

            if run_length >= 130 {
                violations.push(Violation::SignatureLike { run_length });
            } else if run_length >= 64 {
                violations.push(Violation::SecretLike { run_length });
            } else if run_length >= 40 {
                violations.push(Violation::FullAddress { run_length });
            }

            index += 1;
        } else {
            index += 1;
        }
    }

    if runbook.contains("-----BEGIN") {
        violations.push(Violation::PemMarker);
    }

    let mut search_start = 0;
    while let Some(relative_start) = runbook[search_start..].find("Bearer ") {
        let marker_end = search_start + relative_start + "Bearer ".len();
        let token_length = runbook[marker_end..]
            .chars()
            .take_while(|character| !character.is_whitespace())
            .count();
        if token_length >= 20 {
            violations.push(Violation::BearerTokenLike);
        }
        search_start = marker_end;
    }
}

fn documents() -> (String, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runbook = fs::read_to_string(root.join("docs/MANUAL_LIVE_GATE_RUNBOOK.md"))
        .expect("manual live-gate runbook is readable");
    let design = fs::read_to_string(root.join("docs/DEPOSIT_WALLET_RELAYER_DESIGN.md"))
        .expect("deposit-wallet relayer design is readable");
    (runbook, design)
}

fn clean_documents() -> (String, String) {
    let (runbook, design) = documents();
    let violations = audit(&runbook, &design);
    assert!(
        violations.is_empty(),
        "baseline runbook must be clean before mutation: {violations:#?}"
    );
    (runbook, design)
}

fn replace_exactly_once(source: &str, from: &str, to: &str) -> String {
    assert_eq!(
        source.match_indices(from).count(),
        1,
        "synthetic mutation target must occur exactly once: {from}"
    );
    source.replacen(from, to, 1)
}

#[test]
fn manual_live_gate_runbook_satisfies_offline_contract() {
    let (runbook, design) = documents();
    let violations = audit(&runbook, &design);
    assert!(
        violations.is_empty(),
        "manual live-gate runbook audit found violations: {violations:#?}"
    );
}

#[test]
fn audit_rejects_one_removed_section_heading() {
    let (runbook, design) = clean_documents();
    let mutated = replace_exactly_once(
        &runbook,
        "## Stop conditions",
        "## Stop conditions REMOVED",
    );

    assert_eq!(
        audit(&mutated, &design),
        vec![Violation::MissingSection("## Stop conditions")]
    );
}

#[test]
fn audit_rejects_one_removed_stop_condition() {
    let (runbook, design) = clean_documents();
    let mutated = replace_exactly_once(&runbook, "missing transactionID", "");

    assert_eq!(
        audit(&mutated, &design),
        vec![Violation::MissingStopCondition("missing transactionID")]
    );
}

#[test]
fn audit_rejects_one_full_address() {
    let (mut runbook, design) = clean_documents();
    runbook.push_str(&format!("\n0x{}\n", "a".repeat(40)));

    assert_eq!(
        audit(&runbook, &design),
        vec![Violation::FullAddress { run_length: 40 }]
    );
}

#[test]
fn longer_hex_runs_use_only_the_longest_matching_classification() {
    let (runbook, design) = clean_documents();

    for (run_length, expected) in [
        (64, Violation::SecretLike { run_length: 64 }),
        (130, Violation::SignatureLike { run_length: 130 }),
    ] {
        let mut mutated = runbook.clone();
        mutated.push_str(&format!("\n0X{}\n", "b".repeat(run_length)));
        assert_eq!(audit(&mutated, &design), vec![expected]);
    }
}

#[test]
fn overlapping_hex_prefixes_do_not_hide_a_full_address() {
    let (mut runbook, design) = clean_documents();
    runbook.push_str(&format!("\n0x0x{}\n", "c".repeat(40)));

    assert_eq!(
        audit(&runbook, &design),
        vec![Violation::FullAddress { run_length: 40 }]
    );
}

#[test]
fn literal_secret_markers_are_detected_without_echoing_values() {
    let (runbook, design) = clean_documents();

    let mut pem_mutation = runbook.clone();
    pem_mutation.push_str("\n-----BEGIN SYNTHETIC BLOCK\n");
    assert_eq!(
        audit(&pem_mutation, &design),
        vec![Violation::PemMarker]
    );

    let mut bearer_mutation = runbook;
    bearer_mutation.push_str(&format!("\nBearer {}\n", "z".repeat(20)));
    assert_eq!(
        audit(&bearer_mutation, &design),
        vec![Violation::BearerTokenLike]
    );
}
