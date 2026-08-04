//! Offline audit for release provenance, licensing, and accepted advisories.
//!
//! The manifest check proves only that this manifest explicitly selects at
//! least one TLS backend for `ethers`. Cargo features are additive: this does
//! not prove that rustls is absent, that exactly one TLS implementation is
//! resolved, that consumer feature unification cannot reactivate rustls, or
//! that an actual HTTPS connection succeeds.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use yaml_rust2::{Yaml, YamlLoader};

const REQUIRED_SECTIONS: &[&str] = &[
    "## Upstream and fork identity",
    "## Dependency posture",
    "## Accepted advisories",
    "## What this record does not prove",
    "## Publishing decision",
    "## Consumer pinning and rollback",
];

const ADVISORY_HEADER: [&str; 6] = [
    "Advisory",
    "Crate",
    "Path",
    "Scope",
    "Rationale",
    "Re-review condition",
];

const CHECKOUT_ACTION: &str =
    "actions/checkout@11d5960a326750d5838078e36cf38b85af677262";
const INSTALL_ACTION: &str =
    "taiki-e/install-action@67729d5c413db75907f0ad1e39bb04b9c868ff60";
// The preflight cannot check whether it is itself a symlink: Python has
// already opened and executed the link target by the time any check runs. The
// workflow command therefore does that one check before invoking it.
const PREFLIGHT_COMMAND: &str = "test ! -L scripts && test ! -L scripts/preflight_build_integrity.py \
&& python3 -I scripts/preflight_build_integrity.py";
// `cargo audit` reuses an existing lockfile without checking it against the
// manifest, so a committed stale lock would be audited instead of the current
// dependency set. Removing it forces a fresh resolution.
const AUDIT_COMMAND: &str = "rm -f Cargo.lock && cargo audit --deny warnings";

#[derive(Clone, Debug)]
struct Documents {
    manifest: String,
    rust_toolchain_toml: Option<String>,
    extensionless_rust_toolchain_present: bool,
    license_mit: Option<String>,
    license_apache: Option<String>,
    notice: Option<String>,
    provenance: String,
    accepted_advisories: String,
    audit_config: String,
    security_workflow: String,
    cargo_directory_entries: Vec<String>,
    preflight_script: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Violation {
    MissingPublishFalse,
    MalformedManifest,
    MissingEthersDependency,
    MalformedEthersDependency,
    EthersDefaultFeaturesNotFalse,
    EthersOpenSslMissing,
    AutoDiscoveryDisabled { key: &'static str },
    AuditTargetDeclarationMalformed,
    AuditTargetCount { count: usize },
    AuditTargetPathMismatch { actual: Option<String> },
    AuditTargetExcluded { key: &'static str },
    AuditTargetSettingUnsupported { key: String },
    MissingOrEmptyFile(&'static str),
    MissingSection(&'static str),
    DuplicateSection(&'static str),
    UnsupportedConstruct { kind: String },
    AcceptedAdvisoriesHeadingCount { count: usize },
    AdvisoryTableCount { count: usize },
    AdvisoryTableHeaderCount { count: usize },
    AdvisoryTableHeaderMismatch { cells: Vec<String> },
    MalformedAdvisoryRow { cells: Vec<String> },
    InvalidDocumentAdvisory { advisory: String },
    EmptyAdvisoryCell { advisory: String, column: usize },
    DuplicateDocumentAdvisory { advisory: String },
    MalformedAcceptedAdvisoryRegister,
    AcceptedAdvisoryRegisterUnexpectedKey { key: String },
    InvalidAcceptedAdvisoryRegisterCell { index: usize, field: &'static str },
    DuplicateRegisteredAdvisory { advisory: String },
    AdvisoryRenderingMismatch {
        register: Vec<Vec<String>>,
        document: Vec<Vec<String>>,
    },
    MalformedAuditConfig,
    AuditConfigUnexpectedKey { key: String },
    DuplicateConfigAdvisory { advisory: String },
    PositiveVulnerabilityClaim { phrase: &'static str },
    MalformedSecurityAuditWorkflow,
    SecurityAuditTopLevelSchemaMismatch { actual: Vec<String> },
    SecurityAuditTriggerMismatch { actual: Vec<String> },
    SecurityAuditTriggerConfigurationMismatch,
    SecurityAuditPermissionsMismatch,
    SecurityAuditJobsSchemaMismatch { actual: Vec<String> },
    SecurityAuditJobSchemaMismatch { actual: Vec<String> },
    SecurityAuditForbiddenJobKey { key: String },
    SecurityAuditStepCount { count: usize },
    SecurityAuditStepSchemaMismatch { step: usize },
    SecurityAuditForbiddenStepKey { step: usize, key: String },
    SecurityAuditActionMismatch { step: usize, actual: String },
    SecurityAuditCheckoutCredentials,
    SecurityAuditCommandCount { count: usize },
    SecurityAuditCommandMismatch { actual: String },
    SecurityAuditSecretsReference,
    CargoConfigPresent { path: String },
    UnexpectedCargoEntry { path: String },
    CargoAuditConfigEntryCount { count: usize },
    MissingRustToolchainToml,
    ExtensionlessRustToolchainPresent,
    MalformedRustToolchainToml,
    RustToolchainRootSchemaMismatch { key: String },
    RustToolchainSettingUnsupported { key: String },
    ToolchainPathOverride,
    InvalidRustToolchainChannel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProvenanceDocumentView {
    h2_headings: Vec<String>,
    accepted_tables: Vec<AdvisoryTable>,
    rendered_text: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AdvisoryTable {
    rows: Vec<AdvisoryTableRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AdvisoryTableRow {
    cells: Vec<String>,
    is_header: bool,
    has_unsupported_content: bool,
}

#[derive(Debug, Default)]
struct ParseState {
    container_depth: usize,
    list_depth: usize,
    ignored_tag_depth: usize,
    heading_level: Option<HeadingLevel>,
    heading_is_top_level: bool,
    heading_text: String,
    heading_count: usize,
    h1_count: usize,
    text_block: String,
    in_accepted_section: bool,
    table_is_top_level: bool,
    current_table: Option<AdvisoryTable>,
    current_row: Option<AdvisoryTableRow>,
    current_cell: Option<String>,
}

fn audit(documents: &Documents) -> Vec<Violation> {
    let mut violations = Vec::new();

    audit_manifest(&documents.manifest, &mut violations);
    audit_cargo_directory(&documents.cargo_directory_entries, &mut violations);
    audit_rust_toolchain(
        documents.rust_toolchain_toml.as_deref(),
        documents.extensionless_rust_toolchain_present,
        &mut violations,
    );
    audit_security_workflow(&documents.security_workflow, &mut violations);
    audit_required_file(
        "LICENSE-MIT",
        documents.license_mit.as_deref(),
        &mut violations,
    );
    audit_required_file(
        "LICENSE-APACHE",
        documents.license_apache.as_deref(),
        &mut violations,
    );
    audit_required_file("NOTICE", documents.notice.as_deref(), &mut violations);
    audit_required_file(
        "scripts/preflight_build_integrity.py",
        documents.preflight_script.as_deref(),
        &mut violations,
    );
    let view = provenance_document_view(&documents.provenance, &mut violations);
    audit_provenance_sections(&view, &mut violations);

    let document_advisories = document_advisories(&view, &mut violations);
    let registered_advisories =
        accepted_advisories(&documents.accepted_advisories, &mut violations);
    let _ = config_advisories(&documents.audit_config, &mut violations);

    if let (Some(document), Some(register)) = (&document_advisories, &registered_advisories) {
        if document != register {
            violations.push(Violation::AdvisoryRenderingMismatch {
                register: register.clone(),
                document: document.clone(),
            });
        }
    }

    audit_positive_claims(&view, &mut violations);
    violations
}

fn audit_manifest(manifest: &str, violations: &mut Vec<Violation>) {
    let Ok(manifest) = toml::from_str::<toml::Value>(manifest) else {
        violations.push(Violation::MalformedManifest);
        return;
    };

    if manifest
        .get("package")
        .and_then(|package| package.get("publish"))
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        violations.push(Violation::MissingPublishFalse);
    }

    for key in ["autotests", "autobins", "autoexamples", "autobenches"] {
        if manifest
            .get("package")
            .and_then(|package| package.get(key))
            .is_some_and(|value| value.as_bool() != Some(true))
        {
            violations.push(Violation::AutoDiscoveryDisabled { key });
        }
    }

    audit_explicit_release_provenance_target(&manifest, violations);

    let Some(ethers) = manifest
        .get("dependencies")
        .and_then(|dependencies| dependencies.get("ethers"))
    else {
        violations.push(Violation::MissingEthersDependency);
        return;
    };
    let Some(fields) = ethers.as_table() else {
        violations.push(Violation::MalformedEthersDependency);
        return;
    };

    if fields
        .get("default-features")
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        violations.push(Violation::EthersDefaultFeaturesNotFalse);
    }

    let has_openssl = fields
        .get("features")
        .and_then(toml::Value::as_array)
        .is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature.as_str() == Some("openssl"))
        });
    if !has_openssl {
        violations.push(Violation::EthersOpenSslMissing);
    }
}

fn audit_explicit_release_provenance_target(
    manifest: &toml::Value,
    violations: &mut Vec<Violation>,
) {
    let Some(declarations) = manifest.get("test") else {
        return;
    };
    let Some(declarations) = declarations.as_array() else {
        violations.push(Violation::AuditTargetDeclarationMalformed);
        return;
    };
    let Some(declarations) = declarations
        .iter()
        .map(toml::Value::as_table)
        .collect::<Option<Vec<_>>>()
    else {
        violations.push(Violation::AuditTargetDeclarationMalformed);
        return;
    };
    let audit_targets = declarations
        .into_iter()
        .filter(|declaration| {
            declaration.get("name").and_then(toml::Value::as_str)
                == Some("release_provenance_test")
        })
        .collect::<Vec<_>>();
    if audit_targets.len() != 1 {
        violations.push(Violation::AuditTargetCount {
            count: audit_targets.len(),
        });
        return;
    }

    let audit_target = audit_targets[0];
    for key in audit_target.keys().filter(|key| {
        !matches!(
            key.as_str(),
            "name" | "path" | "test" | "harness" | "required-features"
        )
    }) {
        violations.push(Violation::AuditTargetSettingUnsupported { key: key.clone() });
    }

    let actual_path = audit_target
        .get("path")
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if actual_path.as_deref() != Some("tests/release_provenance_test.rs") {
        violations.push(Violation::AuditTargetPathMismatch {
            actual: actual_path,
        });
    }
    for key in ["test", "harness"] {
        if audit_target
            .get(key)
            .is_some_and(|value| value.as_bool() != Some(true))
        {
            violations.push(Violation::AuditTargetExcluded { key });
        }
    }
    if audit_target.get("required-features").is_some_and(|value| {
        value
            .as_array()
            .is_none_or(|required_features| !required_features.is_empty())
    }) {
        violations.push(Violation::AuditTargetExcluded {
            key: "required-features",
        });
    }
}

fn audit_cargo_directory(entries: &[String], violations: &mut Vec<Violation>) {
    let audit_config_count = entries.iter().filter(|entry| *entry == "audit.toml").count();
    if audit_config_count != 1 {
        violations.push(Violation::CargoAuditConfigEntryCount {
            count: audit_config_count,
        });
    }
    for entry in entries.iter().filter(|entry| entry.as_str() != "audit.toml") {
        let path = format!(".cargo/{entry}");
        if matches!(entry.as_str(), "config.toml" | "config") {
            violations.push(Violation::CargoConfigPresent { path });
        } else {
            violations.push(Violation::UnexpectedCargoEntry { path });
        }
    }
}

fn audit_rust_toolchain(
    toolchain_toml: Option<&str>,
    extensionless_present: bool,
    violations: &mut Vec<Violation>,
) {
    if extensionless_present {
        violations.push(Violation::ExtensionlessRustToolchainPresent);
    }

    let Some(toolchain_toml) = toolchain_toml else {
        violations.push(Violation::MissingRustToolchainToml);
        return;
    };
    let Ok(document) = toml::from_str::<toml::Value>(toolchain_toml) else {
        violations.push(Violation::MalformedRustToolchainToml);
        return;
    };
    let Some(root) = document.as_table() else {
        violations.push(Violation::MalformedRustToolchainToml);
        return;
    };
    if root.len() != 1 || !root.contains_key("toolchain") {
        let key = root
            .keys()
            .find(|key| key.as_str() != "toolchain")
            .cloned()
            .unwrap_or_else(|| "<missing:toolchain>".to_owned());
        violations.push(Violation::RustToolchainRootSchemaMismatch { key });
        return;
    }

    let Some(toolchain) = root.get("toolchain").and_then(toml::Value::as_table) else {
        violations.push(Violation::MalformedRustToolchainToml);
        return;
    };

    // `path` has a dedicated violation because, unlike an ordinary unsupported
    // setting, it replaces the executables that are supposed to run this audit.
    if toolchain.contains_key("path") {
        violations.push(Violation::ToolchainPathOverride);
    }
    for key in toolchain.keys().filter(|key| {
        !matches!(
            key.as_str(),
            "channel" | "components" | "targets" | "profile" | "path"
        )
    }) {
        violations.push(Violation::RustToolchainSettingUnsupported { key: key.clone() });
    }

    if toolchain
        .get("channel")
        .and_then(toml::Value::as_str)
        .is_none_or(|channel| channel.trim().is_empty())
    {
        violations.push(Violation::InvalidRustToolchainChannel);
    }
}

fn audit_security_workflow(workflow: &str, violations: &mut Vec<Violation>) {
    let Ok(documents) = YamlLoader::load_from_str(workflow) else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    let [document] = documents.as_slice() else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    let Some(root) = document.as_hash() else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };

    if workflow.contains("secrets.") || yaml_contains_token_reference(document) {
        violations.push(Violation::SecurityAuditSecretsReference);
    }

    let actual_root_keys = yaml_mapping_keys(root);
    if actual_root_keys != ["jobs", "name", "on", "permissions"]
        || yaml_mapping_value(root, "name").and_then(Yaml::as_str) != Some("Security audit")
    {
        violations.push(Violation::SecurityAuditTopLevelSchemaMismatch {
            actual: actual_root_keys,
        });
        return;
    }

    let actual_triggers = yaml_mapping_value(root, "on")
        .and_then(Yaml::as_hash)
        .map(yaml_mapping_keys)
        .unwrap_or_default();
    let expected_triggers = vec!["schedule".to_owned(), "workflow_dispatch".to_owned()];
    if actual_triggers != expected_triggers {
        violations.push(Violation::SecurityAuditTriggerMismatch {
            actual: actual_triggers,
        });
        return;
    }
    let triggers = yaml_mapping_value(root, "on")
        .and_then(Yaml::as_hash)
        .expect("trigger keys were validated above");
    let schedule_is_exact = yaml_mapping_value(triggers, "schedule")
        .and_then(Yaml::as_vec)
        .is_some_and(|schedule| {
            let [entry] = schedule.as_slice() else {
                return false;
            };
            entry.as_hash().is_some_and(|entry| {
                yaml_mapping_keys(entry) == ["cron"]
                    && yaml_mapping_value(entry, "cron").and_then(Yaml::as_str)
                        == Some("17 4 * * *")
            })
        });
    let dispatch_is_exact = yaml_mapping_value(triggers, "workflow_dispatch")
        .is_some_and(Yaml::is_null);
    if !schedule_is_exact || !dispatch_is_exact {
        violations.push(Violation::SecurityAuditTriggerConfigurationMismatch);
        return;
    }

    let permissions_are_exact = yaml_mapping_value(root, "permissions")
        .and_then(Yaml::as_hash)
        .is_some_and(|permissions| {
            yaml_mapping_keys(permissions) == ["contents"]
                && yaml_mapping_value(permissions, "contents").and_then(Yaml::as_str)
                    == Some("read")
        });
    if !permissions_are_exact {
        violations.push(Violation::SecurityAuditPermissionsMismatch);
        return;
    }

    let Some(jobs) = yaml_mapping_value(root, "jobs").and_then(Yaml::as_hash) else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    let actual_job_keys = yaml_mapping_keys(jobs);
    if actual_job_keys != ["security-audit"] {
        violations.push(Violation::SecurityAuditJobsSchemaMismatch {
            actual: actual_job_keys,
        });
        return;
    }
    let Some(job) = yaml_mapping_value(jobs, "security-audit").and_then(Yaml::as_hash) else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    for forbidden in ["permissions", "env", "defaults", "container", "services"] {
        if yaml_mapping_value(job, forbidden).is_some() {
            violations.push(Violation::SecurityAuditForbiddenJobKey {
                key: forbidden.to_owned(),
            });
            return;
        }
    }
    let actual_job_schema = yaml_mapping_keys(job);
    if actual_job_schema != ["name", "runs-on", "steps", "timeout-minutes"]
        || yaml_mapping_value(job, "name").and_then(Yaml::as_str) != Some("cargo audit")
        || yaml_mapping_value(job, "runs-on").and_then(Yaml::as_str) != Some("ubuntu-latest")
        || yaml_mapping_value(job, "timeout-minutes").and_then(Yaml::as_i64) != Some(15)
    {
        violations.push(Violation::SecurityAuditJobSchemaMismatch {
            actual: actual_job_schema,
        });
        return;
    }

    let Some(steps) = yaml_mapping_value(job, "steps").and_then(Yaml::as_vec) else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    if steps.len() != 4 {
        violations.push(Violation::SecurityAuditStepCount { count: steps.len() });
        return;
    }
    let Some(steps) = steps
        .iter()
        .map(Yaml::as_hash)
        .collect::<Option<Vec<_>>>()
    else {
        violations.push(Violation::MalformedSecurityAuditWorkflow);
        return;
    };
    for (index, step) in steps.iter().enumerate() {
        for forbidden in ["continue-on-error", "if"] {
            if yaml_mapping_value(step, forbidden).is_some() {
                violations.push(Violation::SecurityAuditForbiddenStepKey {
                    step: index,
                    key: forbidden.to_owned(),
                });
                return;
            }
        }
    }
    let command_count = steps
        .iter()
        .filter(|step| yaml_mapping_value(step, "run").is_some())
        .count();
    if command_count != 2 {
        violations.push(Violation::SecurityAuditCommandCount {
            count: command_count,
        });
        return;
    }

    let checkout = steps[0];
    if yaml_mapping_keys(checkout) != ["uses", "with"] {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 0 });
        return;
    }
    let checkout_action = yaml_mapping_value(checkout, "uses")
        .and_then(Yaml::as_str)
        .unwrap_or("<non-string>");
    if checkout_action != CHECKOUT_ACTION {
        violations.push(Violation::SecurityAuditActionMismatch {
            step: 0,
            actual: checkout_action.to_owned(),
        });
        return;
    }
    let checkout_with = yaml_mapping_value(checkout, "with").and_then(Yaml::as_hash);
    if checkout_with.is_none_or(|with| yaml_mapping_keys(with) != ["persist-credentials"])
    {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 0 });
        return;
    }
    if checkout_with
        .and_then(|with| yaml_mapping_value(with, "persist-credentials"))
        .and_then(Yaml::as_bool)
        != Some(false)
    {
        violations.push(Violation::SecurityAuditCheckoutCredentials);
        return;
    }

    let preflight = steps[1];
    if yaml_mapping_keys(preflight) != ["run"] {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 1 });
        return;
    }
    // Exact equality, deliberately without `trim()`. `str::trim` strips every
    // Unicode White_Space character, so a trailing U+00A0 would let the
    // workflow invoke `...py\u{00A0}` — a different repository file — while
    // this comparison still reported the reviewed command.
    let actual_preflight = yaml_mapping_value(preflight, "run")
        .and_then(Yaml::as_str)
        .unwrap_or("<non-string>");
    if actual_preflight != PREFLIGHT_COMMAND {
        violations.push(Violation::SecurityAuditCommandMismatch {
            actual: actual_preflight.to_owned(),
        });
        return;
    }

    let install = steps[2];
    if yaml_mapping_keys(install) != ["uses", "with"] {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 2 });
        return;
    }
    let install_action = yaml_mapping_value(install, "uses")
        .and_then(Yaml::as_str)
        .unwrap_or("<non-string>");
    if install_action != INSTALL_ACTION {
        violations.push(Violation::SecurityAuditActionMismatch {
            step: 2,
            actual: install_action.to_owned(),
        });
        return;
    }
    let install_with = yaml_mapping_value(install, "with").and_then(Yaml::as_hash);
    let install_with_is_exact = install_with.is_some_and(|with| {
        yaml_mapping_keys(with) == ["fallback", "tool"]
            && yaml_mapping_value(with, "tool").and_then(Yaml::as_str)
                == Some("cargo-audit@0.22.2")
            && yaml_mapping_value(with, "fallback").and_then(Yaml::as_str) == Some("none")
    });
    if !install_with_is_exact {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 2 });
        return;
    }

    let audit = steps[3];
    if yaml_mapping_keys(audit) != ["run"] {
        violations.push(Violation::SecurityAuditStepSchemaMismatch { step: 3 });
        return;
    }
    // Exact equality; see the preflight comparison above for why `trim()` is
    // not used here either.
    let actual_command = yaml_mapping_value(audit, "run")
        .and_then(Yaml::as_str)
        .unwrap_or("<non-string>");
    if actual_command != AUDIT_COMMAND {
        violations.push(Violation::SecurityAuditCommandMismatch {
            actual: actual_command.to_owned(),
        });
    }
}

fn yaml_mapping_value<'a>(mapping: &'a yaml_rust2::yaml::Hash, key: &str) -> Option<&'a Yaml> {
    mapping.get(&Yaml::String(key.to_owned()))
}

fn yaml_mapping_keys(mapping: &yaml_rust2::yaml::Hash) -> Vec<String> {
    let mut keys = mapping
        .keys()
        .map(|key| key.as_str().unwrap_or("<non-string>").to_owned())
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn yaml_contains_token_reference(value: &Yaml) -> bool {
    match value {
        Yaml::String(value) => {
            let value = value.to_ascii_lowercase();
            ["secrets.", "secrets[", "github.token", "github["]
                .iter()
                .any(|needle| value.contains(needle))
        }
        Yaml::Array(values) => values.iter().any(yaml_contains_token_reference),
        Yaml::Hash(values) => values.iter().any(|(key, value)| {
            yaml_contains_token_reference(key) || yaml_contains_token_reference(value)
        }),
        _ => false,
    }
}

fn audit_required_file(
    name: &'static str,
    contents: Option<&str>,
    violations: &mut Vec<Violation>,
) {
    if contents.is_none_or(|contents| contents.trim().is_empty()) {
        violations.push(Violation::MissingOrEmptyFile(name));
    }
}

fn audit_provenance_sections(view: &ProvenanceDocumentView, violations: &mut Vec<Violation>) {
    for &heading in REQUIRED_SECTIONS {
        let rendered = heading
            .strip_prefix("## ")
            .expect("required section uses an H2 prefix");
        match view
            .h2_headings
            .iter()
            .filter(|candidate| candidate.as_str() == rendered)
            .count()
        {
            0 => violations.push(Violation::MissingSection(heading)),
            1 => {}
            _ => violations.push(Violation::DuplicateSection(heading)),
        }
    }

    for heading in &view.h2_headings {
        let is_allowed = REQUIRED_SECTIONS
            .iter()
            .any(|allowed| allowed.strip_prefix("## ") == Some(heading.as_str()));
        if !is_allowed {
            violations.push(Violation::UnsupportedConstruct {
                kind: "UnexpectedSection".to_owned(),
            });
        }
    }
}

fn document_advisories(
    view: &ProvenanceDocumentView,
    violations: &mut Vec<Violation>,
) -> Option<Vec<Vec<String>>> {
    let accepted_heading_count = view
        .h2_headings
        .iter()
        .filter(|heading| heading.as_str() == "Accepted advisories")
        .count();
    if accepted_heading_count != 1 {
        violations.push(Violation::AcceptedAdvisoriesHeadingCount {
            count: accepted_heading_count,
        });
        return None;
    }
    if view.accepted_tables.len() != 1 {
        violations.push(Violation::AdvisoryTableCount {
            count: view.accepted_tables.len(),
        });
        return None;
    }

    let table = &view.accepted_tables[0];
    let headers: Vec<&AdvisoryTableRow> =
        table.rows.iter().filter(|row| row.is_header).collect();
    if headers.len() != 1 {
        violations.push(Violation::AdvisoryTableHeaderCount {
            count: headers.len(),
        });
        return None;
    }
    let header = headers[0];
    if !header
        .cells
        .iter()
        .map(String::as_str)
        .eq(ADVISORY_HEADER)
    {
        violations.push(Violation::AdvisoryTableHeaderMismatch {
            cells: header.cells.clone(),
        });
        return None;
    }

    let mut advisories = Vec::new();
    let mut rows = Vec::new();
    let mut rows_are_valid = true;
    for row in table.rows.iter().filter(|row| !row.is_header) {
        if row.has_unsupported_content {
            continue;
        }
        if row.cells.len() != ADVISORY_HEADER.len() {
            violations.push(Violation::MalformedAdvisoryRow {
                cells: row.cells.clone(),
            });
            rows_are_valid = false;
            continue;
        }

        if row
            .cells
            .iter()
            .flat_map(|cell| cell.chars())
            .any(|character| !is_allowed_provenance_character(character))
        {
            // Character violations are reported by the single CommonMark
            // parsing pass. Do not also compare a rejected row to the
            // canonical register and emit a misleading drift diagnosis.
            rows_are_valid = false;
        }

        let advisory = row.cells[0].trim().to_owned();
        let mut row_is_empty = false;
        for (column, cell) in row.cells.iter().enumerate() {
            if cell.trim().is_empty() {
                violations.push(Violation::EmptyAdvisoryCell {
                    advisory: advisory.clone(),
                    column,
                });
                row_is_empty = true;
                rows_are_valid = false;
            }
        }
        if row_is_empty {
            continue;
        }

        if !is_rustsec_id(&advisory) {
            violations.push(Violation::InvalidDocumentAdvisory {
                advisory: advisory.clone(),
            });
            rows_are_valid = false;
            continue;
        }
        advisories.push(advisory);
        rows.push(row.cells.clone());
    }

    let duplicate_count_before = violations.len();
    audit_duplicates(
        &advisories,
        |advisory| Violation::DuplicateDocumentAdvisory { advisory },
        violations,
    );
    if violations.len() != duplicate_count_before {
        rows_are_valid = false;
    }
    rows_are_valid.then_some(rows)
}

fn provenance_document_view(
    provenance: &str,
    violations: &mut Vec<Violation>,
) -> ProvenanceDocumentView {
    // Define the complete accepted alphabet instead of trying to enumerate
    // characters that might render invisibly. Future Unicode additions are
    // rejected automatically; widening the document alphabet is an explicit
    // reviewable change.
    let mut reported_disallowed_characters = BTreeSet::new();
    audit_provenance_characters(
        provenance,
        &mut reported_disallowed_characters,
        violations,
    );

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);

    let mut view = ProvenanceDocumentView::default();
    let mut state = ParseState::default();

    for (event, source_range) in Parser::new_ext(provenance, options).into_offset_iter() {
        if let Event::Text(text) | Event::Code(text) = &event {
            // CommonMark character references are decoded in rendered text.
            // Audit the parser output independently from the raw source so an
            // ASCII entity cannot reintroduce a disallowed Unicode character.
            audit_provenance_characters(
                text,
                &mut reported_disallowed_characters,
                violations,
            );
        }
        match event {
            Event::Start(tag) => {
                let allowed = is_allowed_start_tag(&tag);
                if state.ignored_tag_depth > 0 {
                    state.ignored_tag_depth += 1;
                } else if allowed {
                    handle_allowed_start(
                        tag,
                        source_range.start,
                        &mut state,
                        &mut view,
                        violations,
                    );
                } else {
                    violations.push(Violation::UnsupportedConstruct {
                        kind: unsupported_tag_kind(&tag).to_owned(),
                    });
                    mark_current_row_unsupported(&mut state);
                    // Reject unsupported containers as a unit. Discarding all
                    // descendants prevents image alt text and link labels from
                    // entering table cells or rendered-text claims.
                    state.ignored_tag_depth = 1;
                }
            }
            Event::End(tag_end) => {
                if state.ignored_tag_depth > 0 {
                    state.ignored_tag_depth -= 1;
                } else {
                    handle_allowed_end(tag_end, &mut state, &mut view);
                }
            }
            _ if state.ignored_tag_depth > 0 => {}
            Event::Text(text) | Event::Code(text) => append_rendered_text(&text, &mut state),
            Event::SoftBreak => append_rendered_space(&mut state),
            Event::Html(_) => {
                violations.push(Violation::UnsupportedConstruct {
                    kind: "RawHtml".to_owned(),
                });
                mark_current_row_unsupported(&mut state);
            }
            Event::InlineHtml(_) => {
                violations.push(Violation::UnsupportedConstruct {
                    kind: "InlineHtml".to_owned(),
                });
                mark_current_row_unsupported(&mut state);
            }
            Event::HardBreak => violations.push(Violation::UnsupportedConstruct {
                kind: "HardBreak".to_owned(),
            }),
            Event::Rule => violations.push(Violation::UnsupportedConstruct {
                kind: "Rule".to_owned(),
            }),
            Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_) => violations.push(Violation::UnsupportedConstruct {
                kind: "OtherEvent".to_owned(),
            }),
        }
    }

    if state.h1_count == 0 {
        violations.push(Violation::UnsupportedConstruct {
            kind: "Heading1".to_owned(),
        });
    }
    finish_pending_text(&mut state, &mut view);
    view
}

fn is_allowed_start_tag(tag: &Tag<'_>) -> bool {
    // Unlike the live-validation decision document, this provenance record
    // contains no code-fenced usage examples. Its grammar intentionally
    // rejects code blocks instead of exempting their rendered text from
    // normative claim checks.
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Emphasis
            | Tag::Strong
    )
}

fn unsupported_tag_kind(tag: &Tag<'_>) -> &'static str {
    match tag {
        Tag::Image { .. } => "Image",
        Tag::Link { .. } => "Link",
        Tag::CodeBlock(_) => "CodeBlock",
        Tag::HtmlBlock => "RawHtml",
        Tag::BlockQuote(_) => "BlockQuote",
        Tag::FootnoteDefinition(_) => "FootnoteDefinition",
        Tag::MetadataBlock(_) => "MetadataBlock",
        Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
            "DefinitionList"
        }
        _ => "OtherTag",
    }
}

fn handle_allowed_start(
    tag: Tag<'_>,
    source_start: usize,
    state: &mut ParseState,
    view: &mut ProvenanceDocumentView,
    violations: &mut Vec<Violation>,
) {
    match tag {
        Tag::Heading { level, .. } => {
            finish_pending_text(state, view);
            state.heading_is_top_level = state.container_depth == 0;
            if !state.heading_is_top_level {
                violations.push(Violation::UnsupportedConstruct {
                    kind: "HeadingInContainer".to_owned(),
                });
            }
            if state.heading_is_top_level
                && matches!(level, HeadingLevel::H1 | HeadingLevel::H2)
            {
                state.in_accepted_section = false;
            }
            match level {
                HeadingLevel::H1 => {
                    if source_start != 0 || state.heading_count != 0 || state.h1_count != 0 {
                        violations.push(Violation::UnsupportedConstruct {
                            kind: "Heading1".to_owned(),
                        });
                    }
                    state.h1_count += 1;
                }
                HeadingLevel::H2 => {}
                _ => violations.push(Violation::UnsupportedConstruct {
                    kind: "HeadingDepth".to_owned(),
                }),
            }
            state.heading_count += 1;
            state.heading_level = Some(level);
            state.heading_text.clear();
        }
        Tag::Table(_) => {
            finish_pending_text(state, view);
            state.table_is_top_level = state.container_depth == 0;
            if !state.table_is_top_level {
                violations.push(Violation::UnsupportedConstruct {
                    kind: "TableInContainer".to_owned(),
                });
            }
            state.current_table = Some(AdvisoryTable::default());
            state.container_depth += 1;
        }
        Tag::List(_) => {
            state.list_depth += 1;
            if state.list_depth >= 2 {
                violations.push(Violation::UnsupportedConstruct {
                    kind: "NestedList".to_owned(),
                });
            }
            state.container_depth += 1;
        }
        Tag::Item => {
            finish_pending_text(state, view);
            state.container_depth += 1;
        }
        Tag::TableHead => {
            state.container_depth += 1;
            state.current_row = Some(AdvisoryTableRow {
                cells: Vec::new(),
                is_header: true,
                has_unsupported_content: false,
            });
        }
        Tag::TableRow => {
            state.container_depth += 1;
            state.current_row = Some(AdvisoryTableRow {
                cells: Vec::new(),
                is_header: false,
                has_unsupported_content: false,
            });
        }
        Tag::TableCell => {
            state.container_depth += 1;
            state.current_cell = Some(String::new());
        }
        Tag::Paragraph => finish_pending_text(state, view),
        Tag::Emphasis | Tag::Strong => {}
        _ => unreachable!("unsupported tags are filtered before structural handling"),
    }
}

fn handle_allowed_end(
    tag_end: TagEnd,
    state: &mut ParseState,
    view: &mut ProvenanceDocumentView,
) {
    match tag_end {
        TagEnd::Heading(level) => {
            let heading_text = state.heading_text.trim().to_owned();
            if !heading_text.is_empty() {
                view.rendered_text.push(heading_text.clone());
            }
            if state.heading_is_top_level {
                match level {
                    HeadingLevel::H1 => state.in_accepted_section = false,
                    HeadingLevel::H2 => {
                        state.in_accepted_section = heading_text == "Accepted advisories";
                        view.h2_headings.push(heading_text);
                    }
                    _ => {}
                }
            }
            state.heading_is_top_level = false;
            state.heading_level = None;
            state.heading_text.clear();
        }
        TagEnd::Table => {
            if let Some(table) = state.current_table.take() {
                if state.table_is_top_level && state.in_accepted_section {
                    view.accepted_tables.push(table);
                }
            }
            state.table_is_top_level = false;
            state.container_depth -= 1;
        }
        TagEnd::TableHead | TagEnd::TableRow => {
            if let (Some(table), Some(row)) =
                (&mut state.current_table, state.current_row.take())
            {
                table.rows.push(row);
            }
            state.container_depth -= 1;
        }
        TagEnd::TableCell => {
            if let (Some(row), Some(cell)) = (&mut state.current_row, state.current_cell.take()) {
                let cell = cell.trim().to_owned();
                view.rendered_text.push(cell.clone());
                row.cells.push(cell);
            }
            state.container_depth -= 1;
        }
        TagEnd::Paragraph => finish_pending_text(state, view),
        TagEnd::Item => {
            finish_pending_text(state, view);
            state.container_depth -= 1;
        }
        TagEnd::List(_) => {
            state.list_depth -= 1;
            state.container_depth -= 1;
        }
        TagEnd::Emphasis | TagEnd::Strong => {}
        _ => unreachable!("unsupported tag ends are consumed with their rejected starts"),
    }
}

fn append_rendered_text(text: &str, state: &mut ParseState) {
    if let Some(cell) = &mut state.current_cell {
        cell.push_str(text);
    } else if state.heading_level.is_some() {
        state.heading_text.push_str(text);
    } else if state.current_table.is_none() {
        state.text_block.push_str(text);
    }
}

fn append_rendered_space(state: &mut ParseState) {
    if let Some(cell) = &mut state.current_cell {
        append_space(cell);
    } else if state.heading_level.is_some() {
        append_space(&mut state.heading_text);
    } else if state.current_table.is_none() {
        append_space(&mut state.text_block);
    }
}

fn append_space(text: &mut String) {
    if !text.ends_with(' ') {
        text.push(' ');
    }
}

fn finish_pending_text(state: &mut ParseState, view: &mut ProvenanceDocumentView) {
    let text = state.text_block.trim();
    if !text.is_empty() {
        view.rendered_text.push(text.to_owned());
    }
    state.text_block.clear();
}

fn mark_current_row_unsupported(state: &mut ParseState) {
    if let Some(row) = &mut state.current_row {
        row.has_unsupported_content = true;
    }
}

fn is_allowed_provenance_character(character: char) -> bool {
    matches!(character, '\n' | '\u{2192}' | ' '..='~')
}

fn disallowed_character_violation(character: char) -> Violation {
    Violation::UnsupportedConstruct {
        kind: format!("DisallowedCharacter(U+{:04X})", character as u32),
    }
}

fn audit_provenance_characters(
    text: &str,
    reported: &mut BTreeSet<char>,
    violations: &mut Vec<Violation>,
) {
    let disallowed = text
        .chars()
        .filter(|character| !is_allowed_provenance_character(*character))
        .collect::<BTreeSet<_>>();
    for character in disallowed {
        if reported.insert(character) {
            violations.push(disallowed_character_violation(character));
        }
    }
}

fn accepted_advisories(
    register: &str,
    violations: &mut Vec<Violation>,
) -> Option<Vec<Vec<String>>> {
    const FIELDS: [&str; 6] = ["id", "crate", "path", "scope", "rationale", "re-review"];

    let Ok(register) = toml::from_str::<toml::Value>(register) else {
        violations.push(Violation::MalformedAcceptedAdvisoryRegister);
        return None;
    };
    let Some(root) = register.as_table() else {
        violations.push(Violation::MalformedAcceptedAdvisoryRegister);
        return None;
    };
    if root.len() != 1 || !root.contains_key("advisory") {
        let key = root
            .keys()
            .find(|key| key.as_str() != "advisory")
            .cloned()
            .unwrap_or_else(|| "<missing:advisory>".to_owned());
        violations.push(Violation::AcceptedAdvisoryRegisterUnexpectedKey { key });
        return None;
    }
    let Some(entries) = root.get("advisory").and_then(toml::Value::as_array) else {
        violations.push(Violation::MalformedAcceptedAdvisoryRegister);
        return None;
    };

    let mut rows = Vec::new();
    let mut ids = Vec::new();
    let mut valid = true;
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_table() else {
            violations.push(Violation::MalformedAcceptedAdvisoryRegister);
            valid = false;
            continue;
        };
        let expected = FIELDS.iter().copied().collect::<BTreeSet<_>>();
        let actual = entry.keys().map(String::as_str).collect::<BTreeSet<_>>();
        if actual != expected {
            let key = actual
                .symmetric_difference(&expected)
                .next()
                .copied()
                .unwrap_or("<unknown>")
                .to_owned();
            violations.push(Violation::AcceptedAdvisoryRegisterUnexpectedKey { key });
            valid = false;
        }

        let mut cells = Vec::new();
        for field in FIELDS {
            let Some(value) = entry.get(field).and_then(toml::Value::as_str) else {
                violations.push(Violation::InvalidAcceptedAdvisoryRegisterCell {
                    index,
                    field,
                });
                valid = false;
                cells.push(String::new());
                continue;
            };
            if value.trim().is_empty()
                || (field == "id" && !is_rustsec_id(value))
                || (field == "scope" && !matches!(value, "shipped" | "dev-only"))
            {
                violations.push(Violation::InvalidAcceptedAdvisoryRegisterCell {
                    index,
                    field,
                });
                valid = false;
            }
            cells.push(value.to_owned());
        }
        if let Some(advisory) = cells.first() {
            ids.push(advisory.clone());
        }
        rows.push(cells);
    }

    audit_duplicates(
        &ids,
        |advisory| Violation::DuplicateRegisteredAdvisory { advisory },
        violations,
    );
    valid.then_some(rows)
}

fn config_advisories(config: &str, violations: &mut Vec<Violation>) -> Option<Vec<String>> {
    let Ok(config) = toml::from_str::<toml::Value>(config) else {
        violations.push(Violation::MalformedAuditConfig);
        return None;
    };
    let Some(root) = config.as_table() else {
        violations.push(Violation::MalformedAuditConfig);
        return None;
    };
    if root.len() != 1 || !root.contains_key("advisories") {
        let key = root
            .keys()
            .find(|key| key.as_str() != "advisories")
            .cloned()
            .unwrap_or_else(|| "<missing:advisories>".to_owned());
        violations.push(Violation::AuditConfigUnexpectedKey { key });
        return None;
    }
    let Some(advisories) = root.get("advisories").and_then(toml::Value::as_table) else {
        violations.push(Violation::MalformedAuditConfig);
        return None;
    };
    if advisories.len() != 1 || !advisories.contains_key("ignore") {
        let key = advisories
            .keys()
            .find(|key| key.as_str() != "ignore")
            .map(|key| format!("advisories.{key}"))
            .unwrap_or_else(|| "<missing:advisories.ignore>".to_owned());
        violations.push(Violation::AuditConfigUnexpectedKey { key });
        return None;
    }
    let Some(advisories) = advisories
        .get("ignore")
        .and_then(toml::Value::as_array)
        .and_then(|ignore| {
            ignore
                .iter()
                .map(toml::Value::as_str)
                .collect::<Option<Vec<_>>>()
        })
        .filter(|ids| ids.iter().all(|id| is_rustsec_id(id)))
    else {
        violations.push(Violation::MalformedAuditConfig);
        return None;
    };
    let advisories = advisories.into_iter().map(str::to_owned).collect::<Vec<_>>();

    audit_duplicates(
        &advisories,
        |advisory| Violation::DuplicateConfigAdvisory { advisory },
        violations,
    );
    Some(advisories)
}

fn audit_duplicates<F>(values: &[String], violation: F, violations: &mut Vec<Violation>)
where
    F: Fn(String) -> Violation,
{
    let mut counts = BTreeMap::<&str, usize>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    for (value, count) in counts {
        if count > 1 {
            violations.push(violation(value.to_owned()));
        }
    }
}

fn audit_positive_claims(view: &ProvenanceDocumentView, violations: &mut Vec<Violation>) {
    const PHRASES: &[&str] = &[
        "no known vulnerabilities",
        "vulnerability-free",
        "취약점이 없다",
        "취약점 없음",
    ];
    for block in &view.rendered_text {
        let normalized = block
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        for &phrase in PHRASES {
            if normalized.contains(phrase) {
                violations.push(Violation::PositiveVulnerabilityClaim { phrase });
            }
        }
    }
}

fn is_rustsec_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 17
        && value.starts_with("RUSTSEC-")
        && bytes[8..12].iter().all(u8::is_ascii_digit)
        && bytes[12] == b'-'
        && bytes[13..].iter().all(u8::is_ascii_digit)
}

fn documents() -> Documents {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    Documents {
        manifest: read_required(root.join("Cargo.toml")),
        rust_toolchain_toml: read_optional(root.join("rust-toolchain.toml")),
        extensionless_rust_toolchain_present: root.join("rust-toolchain").exists(),
        license_mit: read_optional(root.join("LICENSE-MIT")),
        license_apache: read_optional(root.join("LICENSE-APACHE")),
        notice: read_optional(root.join("NOTICE")),
        provenance: read_required(root.join("docs/RELEASE_PROVENANCE.md")),
        accepted_advisories: read_required(root.join("docs/accepted-advisories.toml")),
        audit_config: read_required(root.join(".cargo/audit.toml")),
        security_workflow: read_required(root.join(".github/workflows/security-audit.yml")),
        cargo_directory_entries: read_directory_entries(root.join(".cargo")),
        preflight_script: read_optional(root.join("scripts/preflight_build_integrity.py")),
    }
}

fn read_required(path: PathBuf) -> String {
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

fn read_optional(path: PathBuf) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_directory_entries(path: PathBuf) -> Vec<String> {
    let mut entries = fs::read_dir(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("{} entry must be readable: {error}", path.display()))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn clean_documents() -> Documents {
    let documents = documents();
    let violations = audit(&documents);
    assert!(
        violations.is_empty(),
        "baseline release provenance must be clean before mutation: {violations:#?}"
    );
    documents
}

fn replace_exactly_once(source: &str, from: &str, to: &str) -> String {
    assert_eq!(
        source.match_indices(from).count(),
        1,
        "synthetic mutation target must occur exactly once: {from}"
    );
    source.replacen(from, to, 1)
}

fn add_registered_advisory(register: &str, advisory: &str) -> String {
    format!(
        "{register}\n[[advisory]]\nid = \"{advisory}\"\ncrate = \"synthetic 1.0.0\"\npath = \"synthetic path\"\nscope = \"shipped\"\nrationale = \"synthetic rationale\"\nre-review = \"synthetic re-review\"\n"
    )
}

fn advisory_ids(rows: &[Vec<String>]) -> Vec<&str> {
    rows.iter()
        .filter_map(|row| row.first().map(String::as_str))
        .collect()
}

static SYNTHETIC_REPOSITORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct SyntheticRepository {
    root: PathBuf,
}

impl SyntheticRepository {
    fn new() -> Self {
        let sequence = SYNTHETIC_REPOSITORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pbrsdk28-preflight-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap_or_else(|error| {
            panic!("synthetic repository {} must be created: {error}", root.display())
        });
        fs::create_dir(root.join(".cargo")).expect("synthetic .cargo directory is created");
        fs::create_dir(root.join(".github")).expect("synthetic .github directory is created");
        fs::create_dir(root.join(".github/workflows"))
            .expect("synthetic workflows directory is created");
        fs::create_dir(root.join("scripts")).expect("synthetic scripts directory is created");
        fs::create_dir(root.join("tests")).expect("synthetic tests directory is created");
        fs::write(root.join(".cargo/audit.toml"), "[advisories]\nignore = []\n")
            .expect("synthetic audit config is written");
        fs::write(
            root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.95.0\"\ncomponents = [\"rustfmt\", \"clippy\"]\n",
        )
        .expect("synthetic toolchain is written");
        fs::write(
            root.join("Cargo.toml"),
            concat!(
                "[package]\n",
                "name = \"synthetic\"\n",
                "version = \"0.0.0\"\n",
                "edition = \"2021\"\n",
                "publish = false\n",
                "\n[dependencies]\n",
                "ethers = { version = \"2\", default-features = false, ",
                "features = [\"openssl\"] }\n",
            ),
        )
        .expect("synthetic manifest is written");
        fs::write(
            root.join("scripts/preflight_build_integrity.py"),
            "# synthetic repository marker\n",
        )
        .expect("synthetic preflight marker is written");
        fs::write(
            root.join("tests/release_provenance_test.rs"),
            "#[test]\nfn synthetic() {}\n",
        )
        .expect("synthetic release provenance test is written");
        fs::create_dir(root.join("docs")).expect("synthetic docs directory is created");
        fs::write(
            root.join("docs/accepted-advisories.toml"),
            concat!(
                "[[advisory]]\n",
                "id = \"RUSTSEC-2025-0134\"\n",
                "crate = \"synthetic 1.0\"\n",
                "path = \"p\"\n",
                "scope = \"shipped\"\n",
                "rationale = \"r\"\n",
                "re-review = \"rr\"\n",
            ),
        )
        .expect("synthetic accepted-advisory register is written");
        fs::write(
            root.join("docs/RELEASE_PROVENANCE.md"),
            concat!(
                "# Synthetic\n\n## Accepted advisories\n\n",
                "| Advisory | Crate | Path | Scope | Rationale | Re-review condition |\n",
                "| --- | --- | --- | --- | --- | --- |\n",
                "| RUSTSEC-2025-0134 | synthetic 1.0 | p | shipped | r | rr |\n",
                "\n## End\n",
            ),
        )
        .expect("synthetic provenance record is written");
        fs::write(
            root.join(".cargo/audit.toml"),
            "[advisories]\nignore = [\n    \"RUSTSEC-2025-0134\",\n]\n",
        )
        .expect("synthetic audit config is rewritten with a matching register");
        // The preflight pins the workflow by digest, so the synthetic copy has
        // to be the reviewed file itself rather than a stand-in.
        for name in [
            "grimoire.yml",
            "rust-validation.yml",
            "security-audit.yml",
        ] {
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows").join(name),
                root.join(".github/workflows").join(name),
            )
            .unwrap_or_else(|error| {
                panic!("reviewed workflow {name} must be copied: {error}")
            });
        }
        for (path, contents) in [
            ("LICENSE-MIT", "synthetic MIT license\n"),
            ("LICENSE-APACHE", "synthetic Apache license\n"),
            ("NOTICE", "synthetic notice\n"),
        ] {
            fs::write(root.join(path), contents).expect("synthetic license file is written");
        }
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for SyntheticRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_preflight(repository: &SyntheticRepository) -> Output {
    let version = Command::new("python3")
        .arg("--version")
        .output()
        .expect("python3 is required to execute the build-integrity preflight tests");
    assert!(
        version.status.success(),
        "python3 --version must succeed for build-integrity preflight tests: {}",
        String::from_utf8_lossy(&version.stderr)
    );

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/preflight_build_integrity.py");
    Command::new("python3")
        .arg("-I")
        .arg(script)
        .current_dir(&repository.root)
        .output()
        .expect("python3 must execute the build-integrity preflight")
}

fn assert_preflight_failure(repository: &SyntheticRepository, reason: &str) {
    let output = run_preflight(repository);
    assert!(
        !output.status.success(),
        "synthetic preflight mutation must fail: {reason}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "preflight failure must explain {reason}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_provenance_satisfies_offline_contract() {
    let documents = documents();
    let violations = audit(&documents);
    assert!(
        violations.is_empty(),
        "release provenance audit found violations: {violations:#?}"
    );
}

#[test]
fn preflight_accepts_a_clean_synthetic_repository() {
    let repository = SyntheticRepository::new();
    let output = run_preflight(&repository);

    assert!(
        output.status.success(),
        "clean synthetic repository must pass preflight: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "preflight build integrity: ok"
    );
}

#[test]
fn isolated_preflight_rejects_a_tomllib_shadow_module() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("scripts/tomllib.py"),
        "import os\nos._exit(0)\n",
    )
    .expect("synthetic tomllib shadow module is written");

    assert_preflight_failure(&repository, "scripts entries must be exactly");
}

#[test]
fn preflight_rejects_an_additional_scripts_entry() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path("scripts/other.py"), "# unexpected\n")
        .expect("synthetic extra script is written");

    assert_preflight_failure(&repository, "scripts entries must be exactly");
}

#[test]
fn preflight_rejects_audit_target_excluded_from_default_tests() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("Cargo.toml"),
        "[package]\nname = \"synthetic\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"release_provenance_test\"\npath = \"tests/release_provenance_test.rs\"\ntest = false\n",
    )
    .expect("synthetic excluded audit target is written");

    assert_preflight_failure(
        &repository,
        "release_provenance_test test must be absent or true",
    );
}

#[test]
fn preflight_rejects_audit_target_with_disabled_harness() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("Cargo.toml"),
        "[package]\nname = \"synthetic\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"release_provenance_test\"\npath = \"tests/release_provenance_test.rs\"\nharness = false\n",
    )
    .expect("synthetic harness-disabled audit target is written");

    assert_preflight_failure(
        &repository,
        "release_provenance_test harness must be absent or true",
    );
}

#[test]
fn preflight_rejects_duplicate_audit_target_declarations() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("Cargo.toml"),
        "[package]\nname = \"synthetic\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"release_provenance_test\"\npath = \"tests/release_provenance_test.rs\"\n\n[[test]]\nname = \"release_provenance_test\"\npath = \"tests/release_provenance_test.rs\"\n",
    )
    .expect("synthetic duplicate audit targets are written");

    assert_preflight_failure(
        &repository,
        "must contain exactly one release_provenance_test entry; found 2",
    );
}

#[test]
fn preflight_rejects_toolchain_path_override() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("rust-toolchain.toml"),
        "[toolchain]\npath = \"./fake\"\n",
    )
    .expect("synthetic toolchain mutation is written");

    assert_preflight_failure(&repository, "toolchain.path is forbidden");
}

#[test]
fn preflight_rejects_extensionless_rust_toolchain() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path("rust-toolchain"), "1.95.0\n")
        .expect("synthetic extensionless toolchain is written");

    assert_preflight_failure(&repository, "rust-toolchain must not exist");
}

#[test]
fn preflight_rejects_repository_cargo_config() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path(".cargo/config.toml"), "[alias]\naudit = \"true\"\n")
        .expect("synthetic Cargo config is written");

    assert_preflight_failure(&repository, ".cargo entries must be exactly");
}

#[test]
fn preflight_rejects_disabled_integration_test_discovery() {
    let repository = SyntheticRepository::new();
    fs::write(
        repository.path("Cargo.toml"),
        "[package]\nname = \"synthetic\"\nversion = \"0.0.0\"\nedition = \"2021\"\nautotests = false\n",
    )
    .expect("synthetic manifest mutation is written");

    assert_preflight_failure(&repository, "package.autotests must be absent or true");
}

#[test]
fn preflight_rejects_missing_release_provenance_test() {
    let repository = SyntheticRepository::new();
    fs::remove_file(repository.path("tests/release_provenance_test.rs"))
        .expect("synthetic release provenance test is removed");

    assert_preflight_failure(
        &repository,
        "tests/release_provenance_test.rs must exist",
    );
}

#[test]
fn cargo_config_toml_is_rejected_from_repository_cargo_directory() {
    let mut documents = clean_documents();
    documents
        .cargo_directory_entries
        .push("config.toml".to_owned());

    assert_eq!(
        audit(&documents),
        vec![Violation::CargoConfigPresent {
            path: ".cargo/config.toml".to_owned(),
        }]
    );
}

#[test]
fn extensionless_cargo_config_is_rejected_from_repository_cargo_directory() {
    let mut documents = clean_documents();
    documents.cargo_directory_entries.push("config".to_owned());

    assert_eq!(
        audit(&documents),
        vec![Violation::CargoConfigPresent {
            path: ".cargo/config".to_owned(),
        }]
    );
}

#[test]
fn any_other_repository_cargo_entry_is_rejected() {
    let mut documents = clean_documents();
    documents
        .cargo_directory_entries
        .push("credentials.toml".to_owned());

    assert_eq!(
        audit(&documents),
        vec![Violation::UnexpectedCargoEntry {
            path: ".cargo/credentials.toml".to_owned(),
        }]
    );
}

#[test]
fn rust_toolchain_path_override_is_rejected_distinctly() {
    let mut documents = clean_documents();
    let toolchain = documents
        .rust_toolchain_toml
        .as_deref()
        .expect("baseline rust-toolchain.toml exists");
    documents.rust_toolchain_toml = Some(replace_exactly_once(
        toolchain,
        "components = [\"rustfmt\", \"clippy\"]\n",
        "components = [\"rustfmt\", \"clippy\"]\npath = \"./fake-toolchain\"\n",
    ));

    assert_eq!(audit(&documents), vec![Violation::ToolchainPathOverride]);
}

#[test]
fn extensionless_rust_toolchain_file_is_rejected() {
    let mut documents = clean_documents();
    documents.extensionless_rust_toolchain_present = true;

    assert_eq!(
        audit(&documents),
        vec![Violation::ExtensionlessRustToolchainPresent]
    );
}

#[test]
fn rust_toolchain_unknown_setting_is_rejected() {
    let mut documents = clean_documents();
    let toolchain = documents
        .rust_toolchain_toml
        .as_deref()
        .expect("baseline rust-toolchain.toml exists");
    documents.rust_toolchain_toml = Some(replace_exactly_once(
        toolchain,
        "components = [\"rustfmt\", \"clippy\"]\n",
        "components = [\"rustfmt\", \"clippy\"]\nunknown = true\n",
    ));

    assert_eq!(
        audit(&documents),
        vec![Violation::RustToolchainSettingUnsupported {
            key: "unknown".to_owned(),
        }]
    );
}

#[test]
fn rust_toolchain_additional_root_table_is_rejected() {
    let mut documents = clean_documents();
    documents
        .rust_toolchain_toml
        .as_mut()
        .expect("baseline rust-toolchain.toml exists")
        .push_str("\n[other]\nvalue = true\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::RustToolchainRootSchemaMismatch {
            key: "other".to_owned(),
        }]
    );
}

#[test]
fn missing_rust_toolchain_toml_is_rejected() {
    let mut documents = clean_documents();
    documents.rust_toolchain_toml = None;

    assert_eq!(
        audit(&documents),
        vec![Violation::MissingRustToolchainToml]
    );
}

#[test]
fn audit_rejects_missing_publish_false() {
    let mut documents = clean_documents();
    documents.manifest = replace_exactly_once(&documents.manifest, "publish = false\n", "");
    assert_eq!(audit(&documents), vec![Violation::MissingPublishFalse]);
}

#[test]
fn audit_rejects_disabled_cargo_auto_discovery() {
    for key in ["autotests", "autobins", "autoexamples", "autobenches"] {
        let mut documents = clean_documents();
        documents.manifest = replace_exactly_once(
            &documents.manifest,
            "publish = false\n",
            &format!("publish = false\n{key} = false\n"),
        );

        assert_eq!(
            audit(&documents),
            vec![Violation::AutoDiscoveryDisabled { key }]
        );
    }
}

#[test]
fn audit_rejects_release_provenance_target_excluded_from_default_tests() {
    let mut documents = clean_documents();
    documents.manifest.push_str(
        "\n[[test]]\nname = \"release_provenance_test\"\npath = \"tests/release_provenance_test.rs\"\ntest = false\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::AuditTargetExcluded { key: "test" }]
    );
}

#[test]
fn audit_rejects_missing_or_empty_preflight_script() {
    for contents in [None, Some(" \n".to_owned())] {
        let mut documents = clean_documents();
        documents.preflight_script = contents;

        assert_eq!(
            audit(&documents),
            vec![Violation::MissingOrEmptyFile(
                "scripts/preflight_build_integrity.py"
            )]
        );
    }
}

#[test]
fn audit_rejects_tls_free_ethers_feature_reduction() {
    let mut documents = clean_documents();
    documents.manifest = replace_exactly_once(
        &documents.manifest,
        "features = [\"openssl\"]",
        "features = []",
    );
    assert_eq!(audit(&documents), vec![Violation::EthersOpenSslMissing]);
}

#[test]
fn toml_multiline_string_cannot_supply_manifest_policy_fields() {
    let mut documents = clean_documents();
    documents.manifest = replace_exactly_once(
        &documents.manifest,
        "ethers = { version = \"2\", default-features = false, features = [\"openssl\"] }",
        "ethers = { version = \"2\", default-features = false, features = [] }",
    );
    documents.manifest = replace_exactly_once(&documents.manifest, "publish = false\n", "");
    documents.manifest = replace_exactly_once(
        &documents.manifest,
        "description = \"A Rust SDK for Polymarket's Builder Relayer — gasless on-chain operations\"",
        "description = \"\"\"\npublish = false\n[dependencies]\nethers = { version = \"2\", default-features = false, features = [\"openssl\"] }\n\"\"\"",
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::MissingPublishFalse,
            Violation::EthersOpenSslMissing,
        ]
    );
}

#[test]
fn quoted_pull_request_trigger_is_rejected_by_yaml_semantics() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        "  workflow_dispatch:\n",
        "  workflow_dispatch:\n  \"pull_request\":\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditTriggerMismatch {
            actual: vec![
                "pull_request".to_owned(),
                "schedule".to_owned(),
                "workflow_dispatch".to_owned(),
            ],
        }]
    );
}

#[test]
fn checkout_comment_cannot_hide_persisted_credentials() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        "persist-credentials: false",
        "persist-credentials: true # persist-credentials: false",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditCheckoutCredentials]
    );
}

#[test]
fn security_audit_rejects_write_permissions() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        "contents: read",
        "contents: write",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditPermissionsMismatch]
    );
}

#[test]
fn security_audit_rejects_job_level_permission_override() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        "  security-audit:\n",
        "  security-audit:\n    permissions:\n      contents: write\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditForbiddenJobKey {
            key: "permissions".to_owned(),
        }]
    );
}

#[test]
fn security_audit_rejects_indexed_secrets_reference() {
    let mut documents = clean_documents();
    let command = "curl -d \"${{ secrets['GITHUB_TOKEN'] }}\" https://example.invalid";
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        AUDIT_COMMAND,
        command,
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::SecurityAuditSecretsReference,
            Violation::SecurityAuditCommandMismatch {
                actual: command.to_owned(),
            },
        ]
    );
}

#[test]
fn security_audit_rejects_indexed_github_token_reference() {
    let mut documents = clean_documents();
    let command = "curl -d \"${{ github['token'] }}\" https://example.invalid";
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        AUDIT_COMMAND,
        command,
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::SecurityAuditSecretsReference,
            Violation::SecurityAuditCommandMismatch {
                actual: command.to_owned(),
            },
        ]
    );
}

#[test]
fn security_audit_rejects_additional_action_step() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        &format!("      - run: {AUDIT_COMMAND}"),
        &format!("      - uses: attacker/example-action@main\n\n      - run: {AUDIT_COMMAND}"),
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditStepCount { count: 5 }]
    );
}

#[test]
fn security_audit_requires_preflight_before_tool_installation() {
    let mut documents = clean_documents();
    let ordered = format!(
        "      - run: {PREFLIGHT_COMMAND}\n\n      - uses: {INSTALL_ACTION} # v2\n        with:\n          tool: cargo-audit@0.22.2\n          fallback: none"
    );
    let reversed = format!(
        "      - uses: {INSTALL_ACTION} # v2\n        with:\n          tool: cargo-audit@0.22.2\n          fallback: none\n\n      - run: {PREFLIGHT_COMMAND}"
    );
    documents.security_workflow =
        replace_exactly_once(&documents.security_workflow, &ordered, &reversed);

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditStepSchemaMismatch { step: 1 }]
    );
}

#[test]
fn security_audit_requires_isolated_python_preflight() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        PREFLIGHT_COMMAND,
        "python3 scripts/preflight_build_integrity.py",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditCommandMismatch {
            actual: "python3 scripts/preflight_build_integrity.py".to_owned(),
        }]
    );
}

#[test]
fn security_audit_rejects_checkout_tag_instead_of_pinned_sha() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        CHECKOUT_ACTION,
        "actions/checkout@v4",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditActionMismatch {
            step: 0,
            actual: "actions/checkout@v4".to_owned(),
        }]
    );
}

#[test]
fn security_audit_rejects_echo_of_required_command() {
    let mut documents = clean_documents();
    let command = format!("echo \"{AUDIT_COMMAND}\"");
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        AUDIT_COMMAND,
        &command,
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditCommandMismatch {
            actual: command.clone(),
        }]
    );
}

#[test]
fn security_audit_rejects_continue_on_error() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        &format!("      - run: {AUDIT_COMMAND}"),
        &format!("      - run: {AUDIT_COMMAND}\n        continue-on-error: true"),
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditForbiddenStepKey {
            step: 3,
            key: "continue-on-error".to_owned(),
        }]
    );
}

#[test]
fn security_audit_rejects_conditional_audit_step() {
    let mut documents = clean_documents();
    documents.security_workflow = replace_exactly_once(
        &documents.security_workflow,
        &format!("      - run: {AUDIT_COMMAND}"),
        &format!("      - run: {AUDIT_COMMAND}\n        if: ${{{{ false }}}}"),
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::SecurityAuditForbiddenStepKey {
            step: 3,
            key: "if".to_owned(),
        }]
    );
}

#[test]
fn audit_rejects_ethers_default_features_drift() {
    let mut documents = clean_documents();
    documents.manifest = replace_exactly_once(
        &documents.manifest,
        "ethers = { version = \"2\", default-features = false, features = [\"openssl\"] }",
        "ethers = { version = \"2\", default-features = true, features = [\"openssl\"] }",
    );
    assert_eq!(
        audit(&documents),
        vec![Violation::EthersDefaultFeaturesNotFalse]
    );
}

#[test]
fn audit_rejects_missing_or_empty_license_and_notice_files() {
    for (name, mutation) in [
        ("LICENSE-MIT", 0_u8),
        ("LICENSE-APACHE", 1_u8),
        ("NOTICE", 2_u8),
    ] {
        let mut documents = clean_documents();
        match mutation {
            0 => documents.license_mit = None,
            1 => documents.license_apache = Some(" \n".to_owned()),
            2 => documents.notice = None,
            _ => unreachable!(),
        }
        assert_eq!(
            audit(&documents),
            vec![Violation::MissingOrEmptyFile(name)]
        );
    }
}

#[test]
fn audit_rejects_one_missing_required_section() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "## Consumer pinning and rollback",
        "## Consumer pinning and rollback REMOVED",
    );
    assert_eq!(
        audit(&documents),
        vec![
            Violation::MissingSection("## Consumer pinning and rollback"),
            Violation::UnsupportedConstruct {
                kind: "UnexpectedSection".to_owned(),
            },
        ]
    );
}

#[test]
fn fenced_raw_heading_cannot_replace_rendered_required_section() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "## Consumer pinning and rollback",
        "## Consumer pinning and rollback REMOVED\n\n```markdown\n## Consumer pinning and rollback\n```",
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::UnsupportedConstruct {
                kind: "CodeBlock".to_owned(),
            },
            Violation::MissingSection("## Consumer pinning and rollback"),
            Violation::UnsupportedConstruct {
                kind: "UnexpectedSection".to_owned(),
            },
        ]
    );
}

#[test]
fn audit_rejects_duplicate_accepted_advisories_heading() {
    let mut documents = clean_documents();
    documents.provenance.push_str("\n## Accepted advisories\n");
    assert_eq!(
        audit(&documents),
        vec![
            Violation::DuplicateSection("## Accepted advisories"),
            Violation::AcceptedAdvisoriesHeadingCount { count: 2 },
        ]
    );
}

#[test]
fn audit_rejects_document_only_advisory() {
    let mut documents = clean_documents();
    let anchor = "| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 |";
    let inserted = "| RUSTSEC-2099-0001 | synthetic 1.0.0 | synthetic path | dev-only | synthetic rationale | synthetic re-review |\n| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 |";
    documents.provenance = replace_exactly_once(&documents.provenance, anchor, inserted);

    assert!(matches!(
        audit(&documents).as_slice(),
        [Violation::AdvisoryRenderingMismatch { register, document }]
            if advisory_ids(document).contains(&"RUSTSEC-2099-0001")
                && !advisory_ids(register).contains(&"RUSTSEC-2099-0001")
    ));
}

#[test]
fn preflight_rejects_config_only_advisory() {
    let repository = SyntheticRepository::new();
    let config = fs::read_to_string(repository.path(".cargo/audit.toml"))
        .expect("synthetic audit config is readable");
    fs::write(
        repository.path(".cargo/audit.toml"),
        replace_exactly_once(
            &config,
            "    \"RUSTSEC-2025-0134\",\n",
            "    \"RUSTSEC-2025-0134\",\n    \"RUSTSEC-2099-0001\",\n",
        ),
    )
    .expect("synthetic audit config-only advisory is written");

    assert_preflight_failure(
        &repository,
        "accepted-advisory register and .cargo/audit.toml ignore list must match",
    );
}

#[test]
fn audit_config_rejects_advisory_settings_outside_closed_schema() {
    let mut documents = clean_documents();
    documents
        .audit_config
        .push_str("severity_threshold = \"critical\"\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::AuditConfigUnexpectedKey {
            key: "advisories.severity_threshold".to_owned(),
        }]
    );
}

#[test]
fn audit_config_rejects_additional_root_tables() {
    let mut documents = clean_documents();
    documents.audit_config.push_str("\n[output]\nformat = \"json\"\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::AuditConfigUnexpectedKey {
            key: "output".to_owned(),
        }]
    );
}

#[test]
fn advisory_id_in_another_section_cannot_satisfy_biconditional() {
    let mut documents = clean_documents();
    let row = "| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 | ethers 2.0.14 → ethers-providers 2.0.14 or ethers-middleware 2.0.14 → reqwest 0.11.27 → rustls-pemfile 1.0.4 | shipped | Unmaintained transitive PEM parser remains in the reqwest 0.11 package graph; this crate does not parse PEM with it directly. | Re-review when ethers or reqwest 0.11 changes, or when PEM parsing becomes direct behavior. |\n";
    documents.provenance = replace_exactly_once(&documents.provenance, row, "");
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "## Dependency posture\n",
        "## Dependency posture\n\nRUSTSEC-2025-0134 appears here only as synthetic prose.\n",
    );

    assert!(matches!(
        audit(&documents).as_slice(),
        [Violation::AdvisoryRenderingMismatch { register, document }]
            if !advisory_ids(document).contains(&"RUSTSEC-2025-0134")
                && advisory_ids(register).contains(&"RUSTSEC-2025-0134")
    ));
}

#[test]
fn audit_rejects_duplicate_document_advisory_before_set_comparison() {
    let mut documents = clean_documents();
    let row = "| RUSTSEC-2025-0057 | fxhash 0.2.1 | ethers 2.0.14 → ethers-providers 2.0.14 → hashers 1.0.1 → fxhash 0.2.1 | shipped | Unmaintained transitive hashing dependency; this crate does not call fxhash directly. | Re-review when ethers-providers or hashers changes, or when fxhash is used directly. |";
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        row,
        &format!("{row}\n{row}"),
    );
    assert_eq!(
        audit(&documents),
        vec![Violation::DuplicateDocumentAdvisory {
            advisory: "RUSTSEC-2025-0057".to_owned()
        }]
    );
}

#[test]
fn audit_rejects_duplicate_config_advisory_before_set_comparison() {
    let mut documents = clean_documents();
    documents.audit_config = replace_exactly_once(
        &documents.audit_config,
        "    \"RUSTSEC-2025-0057\",\n",
        "    \"RUSTSEC-2025-0057\",\n    \"RUSTSEC-2025-0057\",\n",
    );
    assert_eq!(
        audit(&documents),
        vec![Violation::DuplicateConfigAdvisory {
            advisory: "RUSTSEC-2025-0057".to_owned()
        }]
    );
}

#[test]
fn audit_rejects_advisory_row_with_empty_required_cells() {
    let mut documents = clean_documents();
    let row = "| RUSTSEC-2025-0057 | fxhash 0.2.1 | ethers 2.0.14 → ethers-providers 2.0.14 → hashers 1.0.1 → fxhash 0.2.1 | shipped | Unmaintained transitive hashing dependency; this crate does not call fxhash directly. | Re-review when ethers-providers or hashers changes, or when fxhash is used directly. |";
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        row,
        "| RUSTSEC-2025-0057 |  |  |  |  |  |",
    );
    assert_eq!(
        audit(&documents),
        (1..=5)
            .map(|column| Violation::EmptyAdvisoryCell {
                advisory: "RUSTSEC-2025-0057".to_owned(),
                column,
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn html_comment_row_cannot_authorize_config_advisory() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "register is derived.\n\n| Advisory",
        "register is derived.\n\n<!--\n| RUSTSEC-2099-0001 | synthetic 1.0.0 | synthetic path | shipped | synthetic rationale | synthetic re-review |\n-->\n\n| Advisory",
    );
    documents.accepted_advisories =
        add_registered_advisory(&documents.accepted_advisories, "RUSTSEC-2099-0001");

    let violations = audit(&documents);
    assert_eq!(
        violations.first(),
        Some(&Violation::UnsupportedConstruct {
            kind: "RawHtml".to_owned(),
        })
    );
    assert!(matches!(
        violations.get(1),
        Some(Violation::AdvisoryRenderingMismatch { register, document })
            if advisory_ids(register).contains(&"RUSTSEC-2099-0001")
                && !advisory_ids(document).contains(&"RUSTSEC-2099-0001")
    ));
}

#[test]
fn image_alt_text_cannot_authorize_config_advisory() {
    let mut documents = clean_documents();
    let anchor = "| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 |";
    let inserted = "| ![RUSTSEC-2099-0001](icon.svg) | synthetic 1.0.0 | synthetic path | shipped | synthetic rationale | synthetic re-review |\n| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 |";
    documents.provenance = replace_exactly_once(&documents.provenance, anchor, inserted);
    documents.accepted_advisories =
        add_registered_advisory(&documents.accepted_advisories, "RUSTSEC-2099-0001");

    let violations = audit(&documents);
    assert_eq!(
        violations.first(),
        Some(&Violation::UnsupportedConstruct {
            kind: "Image".to_owned(),
        })
    );
    assert!(matches!(
        violations.get(1),
        Some(Violation::AdvisoryRenderingMismatch { register, document })
            if advisory_ids(register).contains(&"RUSTSEC-2099-0001")
                && !advisory_ids(document).contains(&"RUSTSEC-2099-0001")
    ));
}

#[test]
fn fenced_code_row_cannot_authorize_config_advisory() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "register is derived.\n\n| Advisory",
        "register is derived.\n\n```text\n| RUSTSEC-2099-0002 | synthetic 1.0.0 | synthetic path | shipped | synthetic rationale | synthetic re-review |\n```\n\n| Advisory",
    );
    documents.accepted_advisories =
        add_registered_advisory(&documents.accepted_advisories, "RUSTSEC-2099-0002");

    let violations = audit(&documents);
    assert_eq!(
        violations.first(),
        Some(&Violation::UnsupportedConstruct {
            kind: "CodeBlock".to_owned(),
        })
    );
    assert!(matches!(
        violations.get(1),
        Some(Violation::AdvisoryRenderingMismatch { register, document })
            if advisory_ids(register).contains(&"RUSTSEC-2099-0002")
                && !advisory_ids(document).contains(&"RUSTSEC-2099-0002")
    ));
}

#[test]
fn inline_code_advisory_id_is_collected_as_rendered_cell_text() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| `RUSTSEC-2025-0057` | fxhash 0.2.1 |",
    );

    assert!(audit(&documents).is_empty());
}

#[test]
fn hexadecimal_character_reference_cannot_normalize_an_advisory_id() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0&#x200B;057 | fxhash 0.2.1 |",
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::UnsupportedConstruct {
                kind: "DisallowedCharacter(U+200B)".to_owned(),
            },
            Violation::InvalidDocumentAdvisory {
                advisory: "RUSTSEC-2025-0\u{200b}057".to_owned(),
            },
        ]
    );
}

#[test]
fn hexadecimal_character_reference_cannot_fill_a_required_cell() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0057 | &#x2800; |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2800)".to_owned(),
        }]
    );
}

#[test]
fn decimal_character_reference_cannot_normalize_an_advisory_id() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0&#8203;057 | fxhash 0.2.1 |",
    );

    assert_eq!(
        audit(&documents),
        vec![
            Violation::UnsupportedConstruct {
                kind: "DisallowedCharacter(U+200B)".to_owned(),
            },
            Violation::InvalidDocumentAdvisory {
                advisory: "RUSTSEC-2025-0\u{200b}057".to_owned(),
            },
        ]
    );
}

#[test]
fn character_reference_inside_code_span_remains_literal_cell_text() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| `RUSTSEC-2025-0&#x200B;057` | fxhash 0.2.1 |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::InvalidDocumentAdvisory {
            advisory: "RUSTSEC-2025-0&#x200B;057".to_owned(),
        }]
    );
}

#[test]
fn audit_rejects_empty_advisory_column_instead_of_ignoring_row() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "|  | fxhash 0.2.1 |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::EmptyAdvisoryCell {
            advisory: String::new(),
            column: 0,
        }]
    );
}

#[test]
fn zero_width_space_cannot_fill_a_required_advisory_cell() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0057 | \u{200b} |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+200B)".to_owned(),
        }]
    );
}

#[test]
fn invisible_separator_cannot_fill_a_required_advisory_cell() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0057 | \u{2063} |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2063)".to_owned(),
        }]
    );
}

#[test]
fn allowlist_rejects_previously_classified_invisible_characters() {
    for (name, character) in [
        ("FUNCTION APPLICATION", '\u{2061}'),
        ("INVISIBLE TIMES", '\u{2062}'),
        ("INVISIBLE PLUS", '\u{2064}'),
        ("COMBINING GRAPHEME JOINER", '\u{034f}'),
        ("ARABIC LETTER MARK", '\u{061c}'),
        ("VARIATION SELECTOR-1", '\u{fe00}'),
        ("LANGUAGE TAG", '\u{e0001}'),
    ] {
        assert!(!is_allowed_provenance_character(character));
        let mut documents = clean_documents();
        documents
            .provenance
            .push_str(&format!("\nvisible{character}text\n"));
        assert_eq!(
            audit(&documents),
            vec![Violation::UnsupportedConstruct {
                kind: format!("DisallowedCharacter(U+{:04X})", character as u32),
            }],
            "{name} must be rejected by the closed character allowlist"
        );
    }
}

#[test]
fn explicitly_allowed_visible_characters_are_accepted() {
    for character in ['A', '~', '→'] {
        assert!(is_allowed_provenance_character(character));
        let mut documents = clean_documents();
        documents
            .provenance
            .push_str(&format!("\nvisible{character}text\n"));
        assert!(audit(&documents).is_empty());
    }
}

#[test]
fn braille_pattern_blank_is_rejected_by_the_character_allowlist() {
    let mut documents = clean_documents();
    documents.provenance.push_str("\nvisible\u{2800}text\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2800)".to_owned(),
        }]
    );
}

#[test]
fn prior_zero_width_counterexamples_are_rejected_by_the_character_allowlist() {
    for character in ['\u{200b}', '\u{2063}'] {
        let mut documents = clean_documents();
        documents
            .provenance
            .push_str(&format!("\nvisible{character}text\n"));

        assert_eq!(
            audit(&documents),
            vec![Violation::UnsupportedConstruct {
                kind: format!("DisallowedCharacter(U+{:04X})", character as u32),
            }]
        );
    }
}

#[test]
fn braille_pattern_blank_cannot_fill_a_required_advisory_cell() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0057 | \u{2800} |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2800)".to_owned(),
        }]
    );
}

#[test]
fn audit_rejects_malformed_row_appended_to_advisory_table() {
    let mut documents = clean_documents();
    let last_row = "| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 | ethers 2.0.14 → ethers-providers 2.0.14 or ethers-middleware 2.0.14 → reqwest 0.11.27 → rustls-pemfile 1.0.4 | shipped | Unmaintained transitive PEM parser remains in the reqwest 0.11 package graph; this crate does not parse PEM with it directly. | Re-review when ethers or reqwest 0.11 changes, or when PEM parsing becomes direct behavior. |";
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        last_row,
        &format!("{last_row}\n| RUSTSEC-2099-0003 | synthetic 1.0.0 |"),
    );

    assert_eq!(
        audit(&documents),
        (2..=5)
            .map(|column| Violation::EmptyAdvisoryCell {
                advisory: "RUSTSEC-2099-0003".to_owned(),
                column,
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn audit_rejects_non_rustsec_advisory_cell_instead_of_ignoring_row() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| GHSA-synthetic | fxhash 0.2.1 |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::InvalidDocumentAdvisory {
            advisory: "GHSA-synthetic".to_owned(),
        }]
    );
}

#[test]
fn audit_rejects_advisory_table_header_drift() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| Advisory | Crate | Path | Scope | Rationale | Re-review condition |",
        "| Identifier | Crate | Path | Scope | Rationale | Re-review condition |",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::AdvisoryTableHeaderMismatch {
            cells: vec![
                "Identifier".to_owned(),
                "Crate".to_owned(),
                "Path".to_owned(),
                "Scope".to_owned(),
                "Rationale".to_owned(),
                "Re-review condition".to_owned(),
            ],
        }]
    );
}

#[test]
fn rendered_emphasis_cannot_split_positive_vulnerability_claim() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\nThis project has no **known** vulnerabilities.\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::PositiveVulnerabilityClaim {
            phrase: "no known vulnerabilities",
        }]
    );
}

#[test]
fn zero_width_space_cannot_split_positive_vulnerability_claim() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\nThis project has no known vulnerabilit\u{200b}ies.\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+200B)".to_owned(),
        }]
    );
}

#[test]
fn invisible_separator_cannot_split_positive_vulnerability_claim() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\nThis project has no known vulnerabilit\u{2063}ies.\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2063)".to_owned(),
        }]
    );
}

#[test]
fn braille_pattern_blank_cannot_split_positive_vulnerability_claim() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\nThis project has no known vulnerabilit\u{2800}ies.\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "DisallowedCharacter(U+2800)".to_owned(),
        }]
    );
}

#[test]
fn positive_claim_inside_code_block_is_rejected_as_unsupported_syntax() {
    let mut documents = clean_documents();
    documents.provenance.push_str(
        "\n```text\nThis project has no known vulnerabilities.\n```\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "CodeBlock".to_owned(),
        }]
    );
}

#[test]
fn unrelated_negation_does_not_exempt_a_positive_claim_in_the_same_paragraph() {
    let mut documents = clean_documents();
    documents.provenance.push_str(
        "\nThis project has no **known** vulnerabilities. This record does not prove transitive reachability.\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::PositiveVulnerabilityClaim {
            phrase: "no known vulnerabilities",
        }]
    );
}

#[test]
fn negated_quotation_does_not_exempt_an_explicit_positive_claim() {
    let mut documents = clean_documents();
    documents.provenance.push_str(
        "\nThis record does not claim that the project has no known vulnerabilities; nevertheless, the project has no known vulnerabilities.\n",
    );

    assert_eq!(
        audit(&documents),
        vec![Violation::PositiveVulnerabilityClaim {
            phrase: "no known vulnerabilities",
        }]
    );
}

#[test]
fn korean_positive_claim_is_not_exempted_by_unrelated_negation() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\n이 프로젝트는 취약점이 없다. 이 기록은 도달 가능성을 증명하지 않는다.\n");

    let mut expected = [
        0xAC00, 0xAE30, 0xB294, 0xB2A5, 0xB2E4, 0xB2EC, 0xB3C4, 0xB85C, 0xB85D,
        0xBA85, 0xC131, 0xC54A, 0xC57D, 0xC5C6, 0xC740, 0xC744, 0xC774, 0xC810,
        0xC81D, 0xC99D, 0xC9C0, 0xCDE8, 0xD2B8, 0xD504, 0xD558,
    ]
    .map(|codepoint| Violation::UnsupportedConstruct {
        kind: format!("DisallowedCharacter(U+{codepoint:04X})"),
    })
    .to_vec();
    expected.push(Violation::PositiveVulnerabilityClaim {
        phrase: "취약점이 없다",
    });
    assert_eq!(audit(&documents), expected);
}

#[test]
fn positive_claim_inside_html_comment_is_rejected_as_raw_html_not_rendered_text() {
    let mut documents = clean_documents();
    documents
        .provenance
        .push_str("\n<!-- This project has no known vulnerabilities. -->\n");

    assert_eq!(
        audit(&documents),
        vec![Violation::UnsupportedConstruct {
            kind: "RawHtml".to_owned(),
        }]
    );
}

#[test]
fn audit_rejects_every_positive_vulnerability_claim() {
    let documents = clean_documents();

    let mut positive = documents.clone();
    positive
        .provenance
        .push_str("\nThis project has no known vulnerabilities.\n");
    assert_eq!(
        audit(&positive),
        vec![Violation::PositiveVulnerabilityClaim {
            phrase: "no known vulnerabilities"
        }]
    );

    let mut negated = documents;
    negated
        .provenance
        .push_str("\nThis record does not claim that 취약점이 없다.\n");
    let mut expected = [0xB2E4, 0xC57D, 0xC5C6, 0xC774, 0xC810, 0xCDE8]
        .map(|codepoint| Violation::UnsupportedConstruct {
            kind: format!("DisallowedCharacter(U+{codepoint:04X})"),
        })
        .to_vec();
    expected.push(Violation::PositiveVulnerabilityClaim {
        phrase: "취약점이 없다",
    });
    assert_eq!(audit(&negated), expected);
}

/// `str::trim` removes every Unicode White_Space character, not just ASCII
/// blanks. A trailing U+00A0 in the workflow command, paired with a repository
/// file whose name carries the same character, would run a different script
/// while a trimmed comparison still reported the reviewed command.
#[test]
fn workflow_commands_reject_unicode_whitespace_padding() {
    let clean = clean_documents();

    for command in [PREFLIGHT_COMMAND, AUDIT_COMMAND] {
        let padded = format!("\"{command}\u{00A0}\"");
        let mut documents = clean_documents();
        documents.security_workflow =
            replace_exactly_once(&clean.security_workflow, command, &padded);

        assert!(
            audit(&documents).contains(&Violation::SecurityAuditCommandMismatch {
                actual: format!("{command}\u{00A0}"),
            }),
            "padded command must be rejected: {command}"
        );
    }
}

/// A repository can make `.github` a symbolic link: local reads follow it and
/// see a valid workflow, while GitHub Actions declines to treat a linked
/// `.github` as a workflow directory and runs nothing at all. Reading a path
/// never reveals this, so the preflight has to inspect the link itself.
#[test]
fn preflight_rejects_a_symlinked_audited_path() {
    let repository = SyntheticRepository::new();
    let cargo_dir = repository.path(".cargo");
    let relocated = repository.path(".cargo-real");
    fs::rename(&cargo_dir, &relocated).expect("synthetic .cargo is relocated");
    std::os::unix::fs::symlink(".cargo-real", &cargo_dir)
        .expect("synthetic .cargo symlink is created");

    assert_preflight_failure(&repository, "must be a regular path, not a symlink");
}

/// Build scripts run before any test binary is compiled, with the package root
/// as their working directory, so one could rewrite the provenance register
/// and let the in-Cargo audit read a document that was never committed.
#[test]
fn preflight_rejects_an_auto_discovered_build_script() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path("build.rs"), "fn main() {}\n")
        .expect("synthetic build script is written");

    assert_preflight_failure(&repository, "build.rs must not exist");
}

#[test]
fn preflight_rejects_a_declared_build_script() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        manifest.replace("[package]", "[package]\nbuild = \"custom_build.rs\""),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "package.build is forbidden");
}

/// The preflight cannot detect that it is itself a symbolic link: Python has
/// already opened and executed the link target before any check in it runs.
/// Only the workflow command, which is pinned exactly by this audit, can make
/// that check.
/// Asserting the command's spelling proves nothing about how a shell resolves
/// it: `test -L` inspects only the final path component, so a linked `scripts`
/// directory leaves the leaf a regular file. This runs the pinned command.
#[test]
fn preflight_command_rejects_a_linked_scripts_directory() {
    let repository = SyntheticRepository::new();
    let bootstrap = repository.path("bootstrap");
    fs::create_dir_all(&bootstrap).expect("synthetic bootstrap directory is created");
    fs::write(
        bootstrap.join("preflight_build_integrity.py"),
        "print(\"BYPASS\")\n",
    )
    .expect("synthetic replacement script is written");
    fs::remove_dir_all(repository.path("scripts")).expect("synthetic scripts directory is removed");
    std::os::unix::fs::symlink("bootstrap", repository.path("scripts"))
        .expect("synthetic scripts symlink is created");

    let output = Command::new("sh")
        .arg("-c")
        .arg(PREFLIGHT_COMMAND)
        .current_dir(repository.path("."))
        .output()
        .expect("pinned preflight command runs");

    assert!(
        !output.status.success(),
        "a linked scripts directory must fail the pinned command; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn preflight_command_rejects_a_linked_script_file() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path("noop.py"), "print(\"BYPASS\")\n")
        .expect("synthetic replacement script is written");
    let script = repository.path("scripts/preflight_build_integrity.py");
    fs::remove_file(&script).expect("synthetic preflight is removed");
    std::os::unix::fs::symlink("../noop.py", &script)
        .expect("synthetic preflight symlink is created");

    let output = Command::new("sh")
        .arg("-c")
        .arg(PREFLIGHT_COMMAND)
        .current_dir(repository.path("."))
        .output()
        .expect("pinned preflight command runs");

    assert!(
        !output.status.success(),
        "a linked preflight script must fail the pinned command; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// `cargo audit` reuses an existing lockfile without comparing it to the
/// manifest, so a stale lock committed alongside a new dependency would be
/// audited in its place.
#[test]
fn audit_command_discards_any_existing_lockfile() {
    assert!(
        AUDIT_COMMAND.starts_with("rm -f Cargo.lock &&"),
        "audit command must force a fresh dependency resolution"
    );
}

#[test]
fn preflight_rejects_a_local_path_dependency() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        format!("{manifest}\n[dev-dependencies.audit-mutator]\npath = \"audit-mutator\"\n"),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "has unsupported dependency keys");
}

#[test]
fn preflight_rejects_a_patch_table() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        format!("{manifest}\n[patch.crates-io]\nserde = {{ path = \"local-serde\" }}\n"),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "[patch] is forbidden");
}

#[test]
fn preflight_rejects_a_target_specific_path_dependency() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        format!(
            "{manifest}\n[target.'cfg(unix)'.dev-dependencies.audit-mutator]\npath = \"audit-mutator\"\n"
        ),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "has unsupported dependency keys");
}

#[test]
fn preflight_rejects_a_replace_table() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        format!("{manifest}\n[replace]\n\"serde:1.0.0\" = {{ path = \"local-serde\" }}\n"),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "[replace] is forbidden");
}

/// Cargo accepts the underscore spellings as distinct TOML keys, so a local
/// crate can be declared in a table the hyphenated check never reads.
#[test]
fn preflight_rejects_underscore_dependency_table_aliases() {
    for table in [
        "dev_dependencies",
        "build_dependencies",
        "target.'cfg(unix)'.dev_dependencies",
    ] {
        let repository = SyntheticRepository::new();
        let manifest = fs::read_to_string(repository.path("Cargo.toml"))
            .expect("synthetic manifest is readable");
        fs::write(
            repository.path("Cargo.toml"),
            format!("{manifest}\n[{table}.audit-mutator]\npath = \"audit-mutator\"\n"),
        )
        .expect("synthetic manifest is rewritten");

        assert_preflight_failure(&repository, "has unsupported dependency keys");
    }
}

/// `package.workspace` names another workspace root, which need not be a parent
/// directory; that root and its members are outside this audit.
#[test]
fn preflight_rejects_a_declared_workspace_root() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        manifest.replace("[package]", "[package]\nworkspace = \"audit-workspace\""),
    )
    .expect("synthetic manifest is rewritten");

    assert_preflight_failure(&repository, "package.workspace is forbidden");
}

/// Naming the forbidden source keys never finished: `path`, then `git`, then
/// `registry-index`, each able to reach code inside this repository. The
/// dependency check states the small set of keys a reviewed release needs
/// instead, so an unnamed source form is rejected without being enumerated.
#[test]
fn preflight_rejects_every_dependency_source_selector() {
    for source in [
        "git = \"file:///proc/self/cwd\"",
        "registry-index = \"file:///proc/self/cwd\"\nversion = \"1\"",
        "registry = \"internal\"\nversion = \"1\"",
        "path = \"audit-mutator\"",
    ] {
        let repository = SyntheticRepository::new();
        let manifest = fs::read_to_string(repository.path("Cargo.toml"))
            .expect("synthetic manifest is readable");
        fs::write(
            repository.path("Cargo.toml"),
            format!("{manifest}\n[dependencies.audit-mutator]\n{source}\n"),
        )
        .expect("synthetic manifest is rewritten");

        assert_preflight_failure(&repository, "has unsupported dependency keys");
    }
}

/// A nightly channel unlocks unstable manifest features such as
/// `profile-rustflags`, and `-Clinker=` names an executable this repository
/// could also provide. The channel is pinned and the manifest is closed to the
/// sections it actually uses, so neither half of that pair can be assembled.
#[test]
fn preflight_rejects_a_toolchain_channel_other_than_the_pinned_one() {
    let repository = SyntheticRepository::new();
    let toolchain = fs::read_to_string(repository.path("rust-toolchain.toml"))
        .expect("synthetic toolchain file is readable");
    fs::write(
        repository.path("rust-toolchain.toml"),
        toolchain.replace("1.95.0", "nightly"),
    )
    .expect("synthetic toolchain file is rewritten");

    assert_preflight_failure(&repository, "toolchain.channel must be");
}

#[test]
fn preflight_rejects_unsupported_manifest_root_sections() {
    for section in [
        "cargo-features = [\"profile-rustflags\"]",
        "[profile.test]\nrustflags = [\"-Clinker=tools/audit-linker\"]",
        "[profile.dev]\nopt-level = 0",
        "[bench]\nname = \"synthetic\"",
    ] {
        let repository = SyntheticRepository::new();
        let manifest = fs::read_to_string(repository.path("Cargo.toml"))
            .expect("synthetic manifest is readable");
        fs::write(
            repository.path("Cargo.toml"),
            format!("{section}\n{manifest}"),
        )
        .expect("synthetic manifest is rewritten");

        assert_preflight_failure(&repository, "unsupported root sections");
    }
}

/// The in-Cargo audit re-reads these files when it runs, and a library unit
/// test runs before the integration tests with the package root as its working
/// directory. It could add the row that makes an unreviewed ignore look
/// approved. Comparing the committed files in the preflight, before any
/// repository code executes, is what makes that rewrite pointless.
#[test]
fn preflight_compares_the_register_before_repository_code_can_run() {
    let repository = SyntheticRepository::new();
    let config = fs::read_to_string(repository.path(".cargo/audit.toml"))
        .expect("synthetic audit config is readable");
    fs::write(
        repository.path(".cargo/audit.toml"),
        config.replace("ignore = [", "ignore = [\n    \"RUSTSEC-2099-0001\","),
    )
    .expect("synthetic audit config is rewritten");

    assert_preflight_failure(&repository, "ignore list must match");
}

#[test]
fn preflight_rejects_a_register_row_without_a_matching_ignore() {
    let repository = SyntheticRepository::new();
    let register = fs::read_to_string(repository.path("docs/accepted-advisories.toml"))
        .expect("synthetic accepted-advisory register is readable");
    fs::write(
        repository.path("docs/accepted-advisories.toml"),
        add_registered_advisory(&register, "RUSTSEC-2099-0001"),
    )
    .expect("synthetic accepted-advisory register is rewritten");

    assert_preflight_failure(
        &repository,
        "accepted-advisory register and .cargo/audit.toml ignore list must match",
    );
}

#[test]
fn preflight_rejects_unknown_accepted_advisory_register_key() {
    let repository = SyntheticRepository::new();
    let register = fs::read_to_string(repository.path("docs/accepted-advisories.toml"))
        .expect("synthetic accepted-advisory register is readable");
    fs::write(
        repository.path("docs/accepted-advisories.toml"),
        replace_exactly_once(
            &register,
            "id = \"RUSTSEC-2025-0134\"\n",
            "id = \"RUSTSEC-2025-0134\"\nowner = \"nobody\"\n",
        ),
    )
    .expect("synthetic register with an unknown key is written");

    assert_preflight_failure(&repository, "keys must be exactly");
}

#[test]
fn preflight_rejects_empty_accepted_advisory_register_cell() {
    let repository = SyntheticRepository::new();
    let register = fs::read_to_string(repository.path("docs/accepted-advisories.toml"))
        .expect("synthetic accepted-advisory register is readable");
    fs::write(
        repository.path("docs/accepted-advisories.toml"),
        replace_exactly_once(&register, "rationale = \"r\"", "rationale = \"\""),
    )
    .expect("synthetic register with an empty cell is written");

    assert_preflight_failure(&repository, "0.rationale must be a non-empty string");
}

#[test]
fn preflight_rejects_duplicate_accepted_advisory_id() {
    let repository = SyntheticRepository::new();
    let register = fs::read_to_string(repository.path("docs/accepted-advisories.toml"))
        .expect("synthetic accepted-advisory register is readable");
    fs::write(
        repository.path("docs/accepted-advisories.toml"),
        add_registered_advisory(&register, "RUSTSEC-2025-0134"),
    )
    .expect("synthetic register with a duplicate ID is written");

    assert_preflight_failure(&repository, "duplicate advisory IDs");
}

#[test]
fn preflight_rejects_audit_config_severity_threshold() {
    let repository = SyntheticRepository::new();
    let config = fs::read_to_string(repository.path(".cargo/audit.toml"))
        .expect("synthetic audit config is readable");
    fs::write(
        repository.path(".cargo/audit.toml"),
        format!("{config}severity_threshold = \"critical\"\n"),
    )
    .expect("synthetic severity threshold is written");

    assert_preflight_failure(&repository, "advisories keys must be exactly ['ignore']");
}

#[test]
fn preflight_requires_lockfile_removal_in_audit_command() {
    let repository = SyntheticRepository::new();
    let workflow = fs::read_to_string(repository.path(".github/workflows/security-audit.yml"))
        .expect("synthetic security workflow is readable");
    fs::write(
        repository.path(".github/workflows/security-audit.yml"),
        replace_exactly_once(
            &workflow,
            AUDIT_COMMAND,
            "cargo audit --deny warnings",
        ),
    )
    .expect("synthetic audit command without lock removal is written");

    assert_preflight_failure(&repository, "reviewed workflow digest");
}

#[test]
fn preflight_requires_scripts_directory_symlink_check_in_workflow_command() {
    let repository = SyntheticRepository::new();
    let workflow = fs::read_to_string(repository.path(".github/workflows/security-audit.yml"))
        .expect("synthetic security workflow is readable");
    fs::write(
        repository.path(".github/workflows/security-audit.yml"),
        replace_exactly_once(
            &workflow,
            PREFLIGHT_COMMAND,
            "test ! -L scripts/preflight_build_integrity.py && python3 -I scripts/preflight_build_integrity.py",
        ),
    )
    .expect("synthetic preflight command without directory symlink check is written");

    assert_preflight_failure(&repository, "reviewed workflow digest");
}

#[test]
fn preflight_requires_publish_false() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        replace_exactly_once(&manifest, "publish = false\n", ""),
    )
    .expect("synthetic manifest without publish=false is written");

    assert_preflight_failure(&repository, "package.publish must be boolean false");
}

#[test]
fn preflight_requires_ethers_openssl_feature() {
    let repository = SyntheticRepository::new();
    let manifest = fs::read_to_string(repository.path("Cargo.toml"))
        .expect("synthetic manifest is readable");
    fs::write(
        repository.path("Cargo.toml"),
        replace_exactly_once(&manifest, "features = [\"openssl\"]", "features = []"),
    )
    .expect("synthetic manifest without ethers OpenSSL is written");

    assert_preflight_failure(
        &repository,
        "dependencies.ethers.features must include \"openssl\"",
    );
}

#[test]
fn preflight_rejects_empty_notice() {
    let repository = SyntheticRepository::new();
    fs::write(repository.path("NOTICE"), " \n").expect("synthetic NOTICE is emptied");

    assert_preflight_failure(&repository, "NOTICE must not be empty");
}

#[test]
fn preflight_does_not_parse_hidden_markdown_table_rows() {
    let repository = SyntheticRepository::new();
    let provenance = fs::read_to_string(repository.path("docs/RELEASE_PROVENANCE.md"))
        .expect("synthetic provenance is readable");
    fs::write(
        repository.path("docs/RELEASE_PROVENANCE.md"),
        replace_exactly_once(
            &provenance,
            "## Accepted advisories\n",
            "## Accepted advisories\n\n<!--\n| RUSTSEC-2099-0001 | hidden 1.0 | p | shipped | r | rr |\n-->\n",
        ),
    )
    .expect("synthetic hidden Markdown row is written");

    let output = run_preflight(&repository);
    assert!(
        output.status.success(),
        "preflight must ignore non-canonical Markdown: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn advisory_rendering_rejects_changed_cell_value() {
    let mut documents = clean_documents();
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        "| RUSTSEC-2025-0057 | fxhash 0.2.1 |",
        "| RUSTSEC-2025-0057 | fxhash 0.2.2 |",
    );

    assert!(matches!(
        audit(&documents).as_slice(),
        [Violation::AdvisoryRenderingMismatch { register, document }]
            if register[3][1] == "fxhash 0.2.1" && document[3][1] == "fxhash 0.2.2"
    ));
}

#[test]
fn advisory_rendering_rejects_deleted_row() {
    let mut documents = clean_documents();
    let row = "| RUSTSEC-2025-0057 | fxhash 0.2.1 | ethers 2.0.14 → ethers-providers 2.0.14 → hashers 1.0.1 → fxhash 0.2.1 | shipped | Unmaintained transitive hashing dependency; this crate does not call fxhash directly. | Re-review when ethers-providers or hashers changes, or when fxhash is used directly. |\n";
    documents.provenance = replace_exactly_once(&documents.provenance, row, "");

    assert!(matches!(
        audit(&documents).as_slice(),
        [Violation::AdvisoryRenderingMismatch { register, document }]
            if advisory_ids(register).contains(&"RUSTSEC-2025-0057")
                && !advisory_ids(document).contains(&"RUSTSEC-2025-0057")
    ));
}

#[test]
fn advisory_rendering_rejects_row_reordering() {
    let mut documents = clean_documents();
    let first = "| RUSTSEC-2025-0009 | ring 0.16.20 | ethers 2.0.14 → ethers-providers 2.0.14 → jsonwebtoken 8.3.0 → ring 0.16.20 | shipped | Legacy transitive provider dependency; this crate does not directly call jsonwebtoken or ring, and transitive reachability is not established by this record. | Re-review when ethers, ethers-providers, jsonwebtoken, or ring changes; when direct use is added; or when reachability evidence changes. |";
    let second = "| RUSTSEC-2025-0010 | ring 0.16.20 | ethers 2.0.14 → ethers-providers 2.0.14 → jsonwebtoken 8.3.0 → ring 0.16.20 | shipped | Unmaintained legacy transitive provider dependency retained for the current public compatibility surface. | Re-review when the ethers provider stack is upgraded or replaced, or when ring is used directly. |";
    documents.provenance = replace_exactly_once(
        &documents.provenance,
        &format!("{first}\n{second}"),
        &format!("{second}\n{first}"),
    );

    assert!(matches!(
        audit(&documents).as_slice(),
        [Violation::AdvisoryRenderingMismatch { register, document }]
            if advisory_ids(register)[..2] == ["RUSTSEC-2025-0009", "RUSTSEC-2025-0010"]
                && advisory_ids(document)[..2] == ["RUSTSEC-2025-0010", "RUSTSEC-2025-0009"]
    ));
}

/// Checking selected lines only binds what was thought of: the two `run`
/// values can stay verbatim while an `if: ${{ false }}` sibling stops either
/// step from executing. The preflight pins the whole file by digest instead.
#[test]
fn preflight_rejects_any_edit_to_the_reviewed_workflow() {
    for mutation in [
        ("      - run: rm -f Cargo.lock && cargo audit --deny warnings",
         "      - run: rm -f Cargo.lock && cargo audit --deny warnings\n        if: ${{ false }}"),
        ("    timeout-minutes: 15", "    timeout-minutes: 15\n    env:\n      EVIL: 1"),
        ("'17 4 * * *'", "'17 4 31 2 *'"),
    ] {
        let repository = SyntheticRepository::new();
        let workflow = fs::read_to_string(repository.path(".github/workflows/security-audit.yml"))
            .expect("synthetic workflow is readable");
        fs::write(
            repository.path(".github/workflows/security-audit.yml"),
            workflow.replace(mutation.0, mutation.1),
        )
        .expect("synthetic workflow is rewritten");

        assert_preflight_failure(&repository, "reviewed workflow digest");
    }
}

/// Any workflow in this repository can stop the audit workflow from running:
/// one with `permissions: actions: write` can disable it through the Actions
/// API using the automatically provided token. The directory is therefore
/// closed to its reviewed set and every entry is pinned.
#[test]
fn preflight_rejects_changes_to_the_reviewed_workflow_set() {
    let sibling = SyntheticRepository::new();
    fs::write(
        sibling.path(".github/workflows/disable-security-audit.yml"),
        "name: disable\non: [push]\npermissions:\n  actions: write\n",
    )
    .expect("synthetic sibling workflow is written");
    assert_preflight_failure(&sibling, "entries must be exactly");

    let removed = SyntheticRepository::new();
    fs::remove_file(removed.path(".github/workflows/rust-validation.yml"))
        .expect("synthetic workflow is removed");
    assert_preflight_failure(&removed, "entries must be exactly");

    for name in ["grimoire.yml", "rust-validation.yml"] {
        let edited = SyntheticRepository::new();
        let path = edited.path(".github/workflows").join(name);
        let workflow = fs::read_to_string(&path).expect("synthetic workflow is readable");
        fs::write(&path, format!("{workflow}\n# edited\n"))
            .expect("synthetic workflow is rewritten");
        assert_preflight_failure(&edited, "reviewed workflow digest");
    }
}

/// Replacing an audited directory with a gitlink leaves a working tree that
/// looks ordinary to a file-reading check -- right names, right bytes -- while
/// the parent repository's commit tree holds no blobs there at all, so GitHub
/// Actions would find no workflow to run.
#[test]
fn preflight_rejects_an_audited_directory_that_is_a_submodule() {
    for marker in [".github/.git", "docs/.git", "scripts/.git"] {
        let repository = SyntheticRepository::new();
        fs::write(repository.path(marker), "gitdir: ../.git/modules/x\n")
            .expect("synthetic submodule marker is written");

        assert_preflight_failure(&repository, "must be a directory in this repository");
    }

    let with_modules = SyntheticRepository::new();
    fs::write(
        with_modules.path(".gitmodules"),
        "[submodule \"github\"]\n\tpath = .github\n\turl = ../x\n",
    )
    .expect("synthetic gitmodules file is written");

    assert_preflight_failure(&with_modules, ".gitmodules is forbidden");
}

/// `git update-index --cacheinfo 160000` records an audited path as a gitlink
/// while leaving the working tree untouched: right files, right bytes, no
/// `.git` marker, no `.gitmodules`. Only the recorded mode differs, and the
/// parent commit tree then holds no workflow blobs for Actions to find.
#[test]
fn preflight_rejects_an_audited_path_recorded_as_a_gitlink() {
    let repository = SyntheticRepository::new();
    let root = repository.path(".");

    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git runs for the synthetic repository");
        assert!(
            status.status.success(),
            "git {args:?} must succeed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        String::from_utf8_lossy(&status.stdout).trim().to_owned()
    };

    git(&["init", "-q", "."]);
    git(&["config", "user.email", "synthetic@example.invalid"]);
    git(&["config", "user.name", "synthetic"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "synthetic"]);

    let head = git(&["rev-parse", "HEAD"]);
    git(&["rm", "-r", "--cached", "-q", ".github"]);
    git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{head},.github"),
    ]);

    assert_preflight_failure(&repository, "recorded as a gitlink");
}

/// Every other check reads the working tree, but what GitHub Actions runs is
/// the committed tree. `git rm --cached` removes a file from that tree while
/// leaving it in place, so the directory listing, the digests and the schema
/// checks all still pass.
#[test]
fn preflight_rejects_an_untracked_audit_input() {
    let repository = SyntheticRepository::new();
    let root = repository.path(".");

    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git runs for the synthetic repository");
        assert!(
            output.status.success(),
            "git {args:?} must succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    git(&["init", "-q", "."]);
    git(&["config", "user.email", "synthetic@example.invalid"]);
    git(&["config", "user.name", "synthetic"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "synthetic"]);
    git(&["rm", "--cached", "-q", ".github/workflows/security-audit.yml"]);

    assert!(
        repository.path(".github/workflows/security-audit.yml").exists(),
        "the working tree copy must remain in place for this to be a real bypass"
    );
    assert_preflight_failure(&repository, "must be tracked in this repository");
}

/// Presence and mode say nothing about content. Everything after the git
/// checks reads the working tree, so malicious bytes can be staged and the
/// working copy restored: the commit carries one tree while every check sees
/// another.
fn git(repository: &SyntheticRepository, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository.path("."))
        .output()
        .expect("git runs for the synthetic repository");
    assert!(
        output.status.success(),
        "git {args:?} must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_everything(repository: &SyntheticRepository) {
    git(repository, &["init", "-q", "."]);
    git(repository, &["config", "user.email", "synthetic@example.invalid"]);
    git(repository, &["config", "user.name", "synthetic"]);
    git(repository, &["add", "-A"]);
    git(repository, &["commit", "-qm", "synthetic"]);
}

/// Stages hostile bytes and restores the working copy, so the commit carries
/// one tree while every content check reads another.
fn stage_hostile_bytes_and_restore(repository: &SyntheticRepository, relative: &str) {
    let path = repository.path(relative);
    let reviewed = fs::read_to_string(&path).expect("reviewed file is readable");
    fs::write(&path, "hostile\n").expect("hostile bytes are written");
    git(repository, &["add", relative]);
    fs::write(&path, &reviewed).expect("reviewed file is restored");
}

#[test]
fn preflight_rejects_an_index_that_diverges_from_the_working_tree() {
    let repository = SyntheticRepository::new();
    commit_everything(&repository);
    stage_hostile_bytes_and_restore(&repository, ".github/workflows/security-audit.yml");

    assert_preflight_failure(&repository, "differs between the index and the working tree");
}

/// The comparison covers every tracked path rather than a named set, so it has
/// to reject divergence in files no such set ever mentioned. `README.md` and
/// `src/**` are read from disk by the boundary, source-matrix, and
/// no-CLOB-surface tests; while the comparison named its paths, those audits
/// could read bytes the commit does not carry.
#[test]
fn preflight_rejects_worktree_divergence_outside_any_named_audit_path() {
    for relative in ["README.md", "src/lib.rs"] {
        let repository = SyntheticRepository::new();
        fs::create_dir(repository.path("src")).expect("synthetic src directory is created");
        fs::write(repository.path("README.md"), "# synthetic\n")
            .expect("synthetic readme is written");
        fs::write(repository.path("src/lib.rs"), "// synthetic\n")
            .expect("synthetic crate root is written");
        commit_everything(&repository);

        let output = run_preflight(&repository);
        assert!(
            output.status.success(),
            "an agreeing tree must pass before divergence is introduced: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        stage_hostile_bytes_and_restore(&repository, relative);
        assert_preflight_failure(&repository, "differs between the index and the working tree");
    }
}

/// Index/worktree agreement says nothing about a file the index does not hold.
/// `git rm --cached` removes a file from the commit and leaves it on disk, so
/// the comparison has nothing to compare while every audit that walks the tree
/// keeps reading a file the release would not contain.
#[test]
fn preflight_rejects_a_file_removed_from_the_index_but_left_on_disk() {
    let repository = SyntheticRepository::new();
    fs::create_dir(repository.path("src")).expect("synthetic src directory is created");
    fs::write(repository.path("src/lib.rs"), "// synthetic\n")
        .expect("synthetic crate root is written");
    commit_everything(&repository);

    let output = run_preflight(&repository);
    assert!(
        output.status.success(),
        "a tracked tree must pass before the file is removed from the index: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    git(&repository, &["rm", "--cached", "-q", "src/lib.rs"]);
    assert!(
        repository.path("src/lib.rs").is_file(),
        "the working copy must survive, which is what makes this a bypass"
    );

    assert_preflight_failure(&repository, "is untracked");
}

/// What counts as ignored has to come from the tree being audited.
/// `.git/info/exclude` is per-clone and never committed, so if it could silence
/// the untracked check, one uncommitted line plus `git rm --cached` would put a
/// source file back out of sight while the release stopped carrying it.
#[test]
fn preflight_rejects_a_file_hidden_by_an_uncommitted_exclude_file() {
    let repository = SyntheticRepository::new();
    fs::create_dir(repository.path("src")).expect("synthetic src directory is created");
    fs::write(repository.path("src/lib.rs"), "// synthetic\n")
        .expect("synthetic crate root is written");
    commit_everything(&repository);

    let exclude = repository.path(".git/info/exclude");
    fs::create_dir_all(exclude.parent().expect("exclude file has a parent directory"))
        .expect("synthetic git info directory exists");
    let mut patterns = fs::read_to_string(&exclude).unwrap_or_default();
    patterns.push_str("src/lib.rs\n");
    fs::write(&exclude, patterns).expect("uncommitted exclude pattern is written");
    git(&repository, &["rm", "--cached", "-q", "src/lib.rs"]);

    assert_preflight_failure(&repository, "is untracked");
}

/// A file that was never added is equally absent from the commit.
#[test]
fn preflight_rejects_an_untracked_file_the_commit_would_not_carry() {
    let repository = SyntheticRepository::new();
    commit_everything(&repository);
    fs::create_dir(repository.path("src")).expect("synthetic src directory is created");
    fs::write(repository.path("src/lib.rs"), "// never added\n")
        .expect("untracked source file is written");

    assert_preflight_failure(&repository, "is untracked");
}

/// The nested-repository check walks the tree rather than a named set of
/// directories, so `src` is covered even though no such set listed it.
#[test]
fn preflight_rejects_a_nested_repository_outside_any_named_audit_directory() {
    for marker in ["src/.git", "docs/nested/.git"] {
        let repository = SyntheticRepository::new();
        let path = repository.path(marker);
        fs::create_dir_all(path.parent().expect("marker has a parent directory"))
            .expect("synthetic nested directory is created");
        fs::write(path, "gitdir: ../.git/modules/x\n")
            .expect("synthetic submodule marker is written");

        assert_preflight_failure(&repository, "must be a directory in this repository");
    }
}
