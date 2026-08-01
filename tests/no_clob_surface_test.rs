use std::fs;
use std::path::{Path, PathBuf};

const MINIMUM_SOURCE_FILE_COUNT: usize = 40;

const FORBIDDEN_MARKERS: &[&str] = &[
    "POLY_1271",
    "PolyGnosisSafe",
    "maker_amount",
    "makerAmount",
    "taker_amount",
    "takerAmount",
    "fee_rate_bps",
    "feeRateBps",
    "tick_size",
    "tickSize",
    "order_type",
    "orderType",
    "/book",
    "/price",
    "/midpoint",
    "balance-allowance",
    "/cancel-orders",
    "/cancel-all",
    "Poly1271",
    "POLY1271",
    "SignatureType",
];

const FORBIDDEN_FN_DECLARATIONS: &[&str] = &[
    "post_order",
    "create_order",
    "sign_order",
    "cancel_order",
    "cancel_orders",
    "get_order",
    "get_orders",
    "get_book",
    "get_price",
    "get_midpoint",
    "update_balance_allowance",
    "get_balance_allowance",
];

#[derive(Clone, Copy)]
struct AllowedMarker {
    marker: &'static str,
    path: &'static str,
    expected_occurrences: usize,
}

const ALLOWED_MARKERS: &[AllowedMarker] = &[
    AllowedMarker {
        marker: "signature_type",
        path: "src/types.rs",
        expected_occurrences: 8,
    },
    AllowedMarker {
        marker: "signature_type",
        path: "src/direct.rs",
        expected_occurrences: 4,
    },
    AllowedMarker {
        marker: "/orders",
        path: "src/auth/builder.rs",
        expected_occurrences: 1,
    },
    AllowedMarker {
        marker: "rs-clob-client-v2",
        path: "src/deposit_wallet/calldata/config.rs",
        expected_occurrences: 2,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Violation {
    TooFewFiles {
        found: usize,
    },
    MissingAllowedFile {
        marker: String,
        path: String,
    },
    DeadException {
        marker: String,
        path: String,
    },
    OccurrenceCountMismatch {
        marker: String,
        path: String,
        expected: usize,
        found: usize,
    },
    ForbiddenMarker {
        marker: String,
        path: String,
    },
    ForbiddenFnDeclaration {
        name: String,
        path: String,
    },
    OrdersOutsideTestRegion {
        path: String,
    },
}

fn audit(files: &[(String, String)]) -> Vec<Violation> {
    let mut violations = Vec::new();

    if files.len() < MINIMUM_SOURCE_FILE_COUNT {
        violations.push(Violation::TooFewFiles { found: files.len() });
    }

    for allowed in ALLOWED_MARKERS {
        let Some((_, source)) = files.iter().find(|(path, _)| path == allowed.path) else {
            violations.push(Violation::MissingAllowedFile {
                marker: allowed.marker.to_owned(),
                path: allowed.path.to_owned(),
            });
            continue;
        };

        let found = source.match_indices(allowed.marker).count();
        if found == 0 {
            violations.push(Violation::DeadException {
                marker: allowed.marker.to_owned(),
                path: allowed.path.to_owned(),
            });
        } else if found != allowed.expected_occurrences {
            violations.push(Violation::OccurrenceCountMismatch {
                marker: allowed.marker.to_owned(),
                path: allowed.path.to_owned(),
                expected: allowed.expected_occurrences,
                found,
            });
        }
    }

    audit_orders_test_region(files, &mut violations);

    for (path, source) in files {
        for (index, allowed) in ALLOWED_MARKERS.iter().enumerate() {
            if ALLOWED_MARKERS[..index]
                .iter()
                .any(|prior| prior.marker == allowed.marker)
            {
                continue;
            }

            let marker_is_allowed_in_file = ALLOWED_MARKERS
                .iter()
                .any(|candidate| candidate.marker == allowed.marker && candidate.path == path);
            if has_unapproved_case_insensitive_occurrence(
                source,
                allowed.marker,
                marker_is_allowed_in_file,
            ) {
                violations.push(Violation::ForbiddenMarker {
                    marker: allowed.marker.to_owned(),
                    path: path.clone(),
                });
            }
        }

        for (index, marker) in FORBIDDEN_MARKERS.iter().enumerate() {
            // Case-insensitive-equivalent spellings share one canonical
            // diagnostic while remaining explicit entries in the contract.
            if FORBIDDEN_MARKERS[..index]
                .iter()
                .any(|prior| prior.eq_ignore_ascii_case(marker))
            {
                continue;
            }

            if contains_ascii_case_insensitive(source, marker) {
                violations.push(Violation::ForbiddenMarker {
                    marker: (*marker).to_owned(),
                    path: path.clone(),
                });
            }
        }

        for name in FORBIDDEN_FN_DECLARATIONS {
            if contains_fn_declaration(source, name) {
                violations.push(Violation::ForbiddenFnDeclaration {
                    name: (*name).to_owned(),
                    path: path.clone(),
                });
            }
        }
    }

    violations
}

fn audit_orders_test_region(files: &[(String, String)], violations: &mut Vec<Violation>) {
    const PATH: &str = "src/auth/builder.rs";

    let Some((_, source)) = files.iter().find(|(path, _)| path == PATH) else {
        return;
    };
    let Some(orders_index) = source.find("/orders") else {
        return;
    };
    if !matches!(source.find("#[cfg(test)]"), Some(test_index) if test_index < orders_index) {
        violations.push(Violation::OrdersOutsideTestRegion {
            path: PATH.to_owned(),
        });
    }
}

fn contains_ascii_case_insensitive(source: &str, marker: &str) -> bool {
    source
        .as_bytes()
        .windows(marker.len())
        .any(|window| window.eq_ignore_ascii_case(marker.as_bytes()))
}

fn has_unapproved_case_insensitive_occurrence(
    source: &str,
    marker: &str,
    marker_is_allowed_in_file: bool,
) -> bool {
    source
        .as_bytes()
        .windows(marker.len())
        .any(|window| {
            window.eq_ignore_ascii_case(marker.as_bytes())
                && (!marker_is_allowed_in_file || window != marker.as_bytes())
        })
}

fn contains_fn_declaration(source: &str, name: &str) -> bool {
    let mut search_start = 0;

    while let Some(relative_start) = source[search_start..].find("fn") {
        let fn_start = search_start + relative_start;
        let fn_end = fn_start + "fn".len();
        let has_token_start = source[..fn_start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_identifier_character(character));
        let after_fn = &source[fn_end..];
        let (after_trivia, has_separating_trivia) = skip_rust_lexical_trivia(after_fn);

        if has_token_start && has_separating_trivia {
            let candidate = after_trivia.strip_prefix("r#").unwrap_or(after_trivia);
            if let Some(after_name) = candidate.strip_prefix(name) {
                let has_token_end = after_name
                    .chars()
                    .next()
                    .is_none_or(|character| !is_identifier_character(character));
                if has_token_end {
                    return true;
                }
            }
        }

        search_start = fn_end;
    }

    false
}

fn skip_rust_lexical_trivia(mut source: &str) -> (&str, bool) {
    let mut skipped = false;

    loop {
        let after_whitespace = source.trim_start_matches(is_rust_lexical_whitespace);
        if after_whitespace.len() < source.len() {
            source = after_whitespace;
            skipped = true;
            continue;
        }

        if starts_non_doc_line_comment(source) {
            source = match source.find('\n') {
                Some(newline) => &source[newline + 1..],
                None => "",
            };
            skipped = true;
            continue;
        }

        if source.starts_with("/*") {
            let Some(after_comment) = strip_nested_block_comment(source) else {
                return ("", true);
            };
            source = after_comment;
            skipped = true;
            continue;
        }

        return (source, skipped);
    }
}

fn is_rust_lexical_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

fn starts_non_doc_line_comment(source: &str) -> bool {
    let Some(after_slashes) = source.strip_prefix("//") else {
        return false;
    };

    after_slashes.starts_with("//")
        || (!after_slashes.starts_with('/') && !after_slashes.starts_with('!'))
}

fn strip_nested_block_comment(source: &str) -> Option<&str> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut index = "/*".len();

    while index + 1 < bytes.len() {
        match (bytes[index], bytes[index + 1]) {
            (b'/', b'*') => {
                depth += 1;
                index += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return Some(&source[index..]);
                }
            }
            _ => index += 1,
        }
    }

    None
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn source_corpus() -> Vec<(String, String)> {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    collect_rust_sources(&repository_root.join("src"), &mut paths);
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(repository_root)
                .unwrap_or_else(|error| {
                    panic!(
                        "source path {} must be inside repository root: {error}",
                        path.display()
                    )
                })
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("Rust source {} must be readable: {error}", path.display())
            });
            (relative, source)
        })
        .collect()
}

fn collect_rust_sources(directory: &Path, paths: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        panic!(
            "Rust source directory {} must be readable: {error}",
            directory.display()
        )
    });

    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "Rust source entry under {} must be readable: {error}",
                directory.display()
            )
        });
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!(
                "Rust source entry {} must have readable metadata: {error}",
                entry.path().display()
            )
        });
        let path = entry.path();

        if file_type.is_dir() {
            collect_rust_sources(&path, paths);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            paths.push(path);
        }
    }
}

#[test]
fn repository_source_has_no_reviewed_clob_markers_or_function_declarations() {
    let files = source_corpus();
    assert!(
        files.len() >= MINIMUM_SOURCE_FILE_COUNT,
        "source audit must inspect at least {MINIMUM_SOURCE_FILE_COUNT} Rust files, found {}",
        files.len()
    );

    let violations = audit(&files);
    assert!(
        violations.is_empty(),
        "reviewed no-CLOB source audit found violations: {violations:#?}"
    );
}

#[test]
fn no_clob_audit_lists_are_not_empty() {
    assert!(FORBIDDEN_MARKERS.len() >= 18);
    assert!(FORBIDDEN_FN_DECLARATIONS.len() >= 10);
}

#[test]
fn no_clob_audit_rejects_allowed_marker_count_growth() {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/auth/builder.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain the conditionally allowed /orders source");
    source.push_str("\nconst SYNTHETIC_ORDER_PATH: &str = \"/orders\";\n");

    assert_eq!(
        audit(&mutated),
        vec![Violation::OccurrenceCountMismatch {
            marker: "/orders".to_owned(),
            path: path.to_owned(),
            expected: 1,
            found: 2,
        }]
    );
}

#[test]
fn no_clob_audit_rejects_orders_before_test_region() {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/auth/builder.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain the conditionally allowed /orders source");
    assert_eq!(source.match_indices("/orders").count(), 1);
    let without_orders = source.replacen("/orders", "", 1);
    *source = format!("/orders\n{without_orders}");
    assert_eq!(source.match_indices("/orders").count(), 1);

    assert_eq!(
        audit(&mutated),
        vec![Violation::OrdersOutsideTestRegion {
            path: path.to_owned(),
        }]
    );
}

#[test]
fn no_clob_audit_rejects_signature_type_markers() {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/lib.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain an arbitrary source for synthetic mutation");
    source.push_str("\npub enum SignatureType { Poly1271 }\n");

    assert_eq!(
        audit(&mutated),
        vec![
            Violation::ForbiddenMarker {
                marker: "Poly1271".to_owned(),
                path: path.to_owned(),
            },
            Violation::ForbiddenMarker {
                marker: "SignatureType".to_owned(),
                path: path.to_owned(),
            },
        ]
    );
}

#[test]
fn no_clob_audit_rejects_private_function_declaration_with_irregular_spacing() {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/lib.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain an arbitrary source for synthetic mutation");
    source.push_str("\npub(crate)  fn  post_order() {}\n");

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenFnDeclaration {
            name: "post_order".to_owned(),
            path: path.to_owned(),
        }]
    );
}

fn assert_post_order_declaration_is_forbidden(declaration: &str) {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/lib.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain an arbitrary source for synthetic mutation");
    source.push_str(declaration);

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenFnDeclaration {
            name: "post_order".to_owned(),
            path: path.to_owned(),
        }]
    );
}

#[test]
fn no_clob_audit_rejects_block_comment_trivia() {
    assert_post_order_declaration_is_forbidden("\nfn/*gap*/post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_line_comment_trivia() {
    assert_post_order_declaration_is_forbidden("\nfn // gap\npost_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_repeated_block_comment_trivia() {
    assert_post_order_declaration_is_forbidden("\nfn /*a*/ /*b*/ post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_nested_block_comment_trivia() {
    assert_post_order_declaration_is_forbidden(
        "\nfn /* outer /* nested */ outer */ post_order() {}\n",
    );
}

#[test]
fn no_clob_audit_rejects_mixed_repeated_trivia() {
    assert_post_order_declaration_is_forbidden(
        "\nfn /*a*/ // b\n /*c*/ post_order() {}\n",
    );
}

#[test]
fn no_clob_audit_rejects_raw_identifier_function_name() {
    assert_post_order_declaration_is_forbidden("\nfn r#post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_horizontal_tab_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{0009}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_line_feed_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{000A}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_vertical_tab_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{000B}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_form_feed_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{000C}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_carriage_return_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{000D}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_space_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{0020}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_next_line_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{0085}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_left_to_right_mark_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{200E}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_right_to_left_mark_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{200F}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_line_separator_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{2028}post_order() {}\n");
}

#[test]
fn no_clob_audit_rejects_paragraph_separator_whitespace() {
    assert_post_order_declaration_is_forbidden("\nfn\u{2029}post_order() {}\n");
}

#[test]
fn no_clob_audit_does_not_match_a_longer_function_identifier() {
    let baseline = source_corpus();
    let baseline_violations = audit(&baseline);
    assert!(
        baseline_violations.is_empty(),
        "baseline source corpus must be clean: {baseline_violations:#?}"
    );

    let mut mutated = baseline.clone();
    let path = "src/lib.rs";
    let source = mutated
        .iter_mut()
        .find(|(candidate, _)| candidate == path)
        .map(|(_, source)| source)
        .expect("baseline corpus must contain an arbitrary source for synthetic mutation");
    source.push_str("\nfn post_order_v2() {}\n");

    let violations = audit(&mutated);
    assert!(
        violations.is_empty(),
        "longer function identifiers must not match post_order: {violations:#?}"
    );
}
