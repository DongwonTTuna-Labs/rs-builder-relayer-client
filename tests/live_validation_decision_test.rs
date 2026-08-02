//! Offline audit for `docs/LIVE_VALIDATION_DECISION.md`.
//!
//! This audit does not prove that conditional-section prose matches the status,
//! that blockers exist or cite a GATE, that non-canonical prose matches the
//! status, or that fenced examples are normative. It also does not compare the
//! README, establish that filled fields are real observations, establish that
//! blocker claims are true, inspect evidence outside this repository, or detect
//! unprefixed hexadecimal, UUID, or base64 forms.

use std::fs;
use std::path::Path;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const STATUS_PREFIX: &str = "<!-- live-validation-status: ";
const STATUS_CANDIDATE_PREFIX: &str = "<!-- live-validation-status:";
const STATUS_SUFFIX: &str = " -->";

const ALWAYS_REQUIRED_SECTIONS: &[&str] = &[
    "## Decision",
    "## What this means for the release",
    "## Existing evidence for the disabled default",
    "## Operator evidence template",
    "## What this record does not prove",
    "## How to supersede this record",
];

const CONDITIONAL_SECTIONS: &[&str] = &[
    "## What was not done",
    "## Blockers",
    "## Why the run was stopped",
];

const EVIDENCE_LABELS: &[&str] = &[
    "operator approval reference",
    "approved network",
    "approved max amount",
    "approved market",
    "approved call type",
    "fresh nonce check",
    "permit mode and scope",
    "permit expiry",
    "submit time",
    "transactionID",
    "terminal state",
    "balance and allowance observation",
    "rollback state",
    "final verdict",
];

const BLOCKED_DECISION: &str = "Live validation was not performed.";
const STOPPED_DECISION: &str =
    "Live validation was started and no confirmation was observed.";
const EXECUTED_DECISION: &str =
    "Live validation reached a confirmed transaction state.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Blocked,
    Stopped,
    Executed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Stopped => "stopped",
            Self::Executed => "executed",
        }
    }

    fn decision(self) -> &'static str {
        match self {
            Self::Blocked => BLOCKED_DECISION,
            Self::Stopped => STOPPED_DECISION,
            Self::Executed => EXECUTED_DECISION,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EvidenceValueRule {
    MustBeUnfilled,
    MustBeFilled,
    InvalidTerminalState,
    InvalidFinalVerdict,
    NotReachedForbidden,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Violation {
    MissingStatusMarker,
    MultipleStatusMarkers { count: usize },
    MalformedStatusMarker { line: String },
    UnknownStatusMarker { value: String },
    MissingSection(&'static str),
    DuplicateSection(&'static str),
    UnexpectedSection { heading: String },
    UnsupportedConstruct { kind: String },
    ForbiddenSection(&'static str),
    DecisionStatusMismatch { status: &'static str },
    EvidenceTableCount { count: usize },
    EvidenceHeaderMismatch { cells: Vec<String> },
    EvidenceMalformedRow { label: String },
    EvidenceUnexpectedLabel { label: String },
    EvidenceLabelMissing(&'static str),
    EvidenceLabelDuplicate { label: &'static str, count: usize },
    EvidenceOrderMismatch {
        expected: &'static str,
        found: String,
    },
    InvalidEvidenceValue {
        label: &'static str,
        rule: EvidenceValueRule,
    },
    FullAddress { run_length: usize },
    PrivateKeyLike { run_length: usize },
    SignatureLike { run_length: usize },
    PemMarker,
    BearerMarker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentView {
    sections: Vec<Section>,
    html_lines: Vec<String>,
    rendered_text: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Section {
    heading: String,
    paragraphs: Vec<String>,
    tables: Vec<Vec<TableRowCells>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableRowCells {
    cells: Vec<String>,
    is_header: bool,
}

struct ParseState {
    current_section: Option<usize>,
    in_code_block: bool,
    heading_level: Option<HeadingLevel>,
    heading_text: String,
    text_block: String,
    current_table: Option<Vec<TableRowCells>>,
    current_row: Option<TableRowCells>,
    current_cell: Option<String>,
    ignored_tag_depth: usize,
    heading_count: usize,
    h1_count: usize,
    list_depth: usize,
    container_depth: usize,
    heading_is_top_level: bool,
    table_is_top_level: bool,
    in_paragraph: bool,
    paragraph_is_top_level: bool,
}

fn audit(doc: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let view = document_view(doc, &mut violations);
    let status = audit_status_marker(&view, &mut violations);

    audit_sections(&view, status, &mut violations);
    if let Some(status) = status {
        audit_decision(&view, status, &mut violations);
    }
    audit_evidence_table(&view, status, &mut violations);
    audit_secret_shapes(doc, &mut violations);

    violations
}

fn document_view(doc: &str, violations: &mut Vec<Violation>) -> DocumentView {
    let mut view = DocumentView {
        sections: Vec::new(),
        html_lines: Vec::new(),
        rendered_text: Vec::new(),
    };
    let mut state = ParseState {
        current_section: None,
        in_code_block: false,
        heading_level: None,
        heading_text: String::new(),
        text_block: String::new(),
        current_table: None,
        current_row: None,
        current_cell: None,
        ignored_tag_depth: 0,
        heading_count: 0,
        h1_count: 0,
        list_depth: 0,
        container_depth: 0,
        heading_is_top_level: false,
        table_is_top_level: false,
        in_paragraph: false,
        paragraph_is_top_level: false,
    };

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);

    for (event, source_range) in Parser::new_ext(doc, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let allowed = is_allowed_start_tag(&tag);
                if !allowed {
                    violations.push(Violation::UnsupportedConstruct {
                        kind: unsupported_tag_kind(&tag).to_owned(),
                    });
                }

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
                    // Unsupported containers are rejected as a unit. Ignoring
                    // their descendants also prevents image alt text or link
                    // labels from becoming normative Decision or table text.
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
            Event::Text(text) | Event::Code(text) if !state.in_code_block => {
                append_text(&text, &mut state);
            }
            Event::Text(_) | Event::Code(_) => {}
            Event::SoftBreak if !state.in_code_block => append_soft_break(&mut state),
            Event::SoftBreak => {}
            Event::Html(html) => {
                for line in html.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with(STATUS_CANDIDATE_PREFIX) {
                        view.html_lines.push(line.to_owned());
                    } else {
                        violations.push(Violation::UnsupportedConstruct {
                            kind: "RawHtml".to_owned(),
                        });
                    }
                }
            }
            Event::InlineHtml(_) => violations.push(Violation::UnsupportedConstruct {
                kind: "InlineHtml".to_owned(),
            }),
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
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::CodeBlock(_)
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::HtmlBlock
    )
}

fn unsupported_tag_kind(tag: &Tag<'_>) -> &'static str {
    match tag {
        Tag::Image { .. } => "Image",
        Tag::Link { .. } => "Link",
        Tag::Emphasis => "Emphasis",
        Tag::Strong => "Strong",
        Tag::BlockQuote(_) => "BlockQuote",
        Tag::FootnoteDefinition(_) => "FootnoteDefinition",
        Tag::MetadataBlock(_) => "MetadataBlock",
        Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
            "DefinitionList"
        }
        Tag::Paragraph
        | Tag::Heading { .. }
        | Tag::CodeBlock(_)
        | Tag::HtmlBlock
        | Tag::List(_)
        | Tag::Item
        | Tag::Table(_)
        | Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::Strikethrough
        | Tag::Superscript
        | Tag::Subscript => "OtherTag",
    }
}

fn handle_allowed_start(
    tag: Tag<'_>,
    source_start: usize,
    state: &mut ParseState,
    view: &mut DocumentView,
    violations: &mut Vec<Violation>,
) {
    match tag {
        Tag::CodeBlock(_) => {
            finish_pending_text(state, view);
            state.in_code_block = true;
        }
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
                state.current_section = None;
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
            state.current_table = Some(Vec::new());
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
            state.current_row = Some(TableRowCells {
                cells: Vec::new(),
                is_header: true,
            });
        }
        Tag::TableRow => {
            state.container_depth += 1;
            state.current_row = Some(TableRowCells {
                cells: Vec::new(),
                is_header: false,
            });
        }
        Tag::TableCell => {
            state.container_depth += 1;
            state.current_cell = Some(String::new());
        }
        Tag::Paragraph => {
            finish_pending_text(state, view);
            state.in_paragraph = true;
            state.paragraph_is_top_level = state.container_depth == 0;
        }
        Tag::HtmlBlock => {}
        _ => unreachable!("unsupported tags are filtered before structural handling"),
    }
}

fn handle_allowed_end(tag_end: TagEnd, state: &mut ParseState, view: &mut DocumentView) {
    match tag_end {
        TagEnd::CodeBlock => state.in_code_block = false,
        TagEnd::Heading(level) => {
            let heading_text = state.heading_text.trim().to_owned();
            if !heading_text.is_empty() {
                view.rendered_text.push(heading_text.clone());
            }
            if level == HeadingLevel::H2 && state.heading_is_top_level {
                view.sections.push(Section {
                    heading: heading_text,
                    paragraphs: Vec::new(),
                    tables: Vec::new(),
                });
                state.current_section = Some(view.sections.len() - 1);
            }
            state.heading_is_top_level = false;
            state.heading_level = None;
            state.heading_text.clear();
        }
        TagEnd::Table => {
            if let Some(table) = state.current_table.take() {
                if let Some(section_index) = state.current_section.filter(|_| state.table_is_top_level)
                {
                    view.sections[section_index].tables.push(table);
                }
            }
            state.table_is_top_level = false;
            state.container_depth -= 1;
        }
        TagEnd::TableHead => {
            if let (Some(table), Some(row)) =
                (&mut state.current_table, state.current_row.take())
            {
                table.push(row);
            }
            state.container_depth -= 1;
        }
        TagEnd::TableRow => {
            if let (Some(table), Some(row)) =
                (&mut state.current_table, state.current_row.take())
            {
                table.push(row);
            }
            state.container_depth -= 1;
        }
        TagEnd::TableCell => {
            if let (Some(row), Some(cell)) =
                (&mut state.current_row, state.current_cell.take())
            {
                let cell = cell.trim().to_owned();
                view.rendered_text.push(cell.clone());
                row.cells.push(cell);
            }
            state.container_depth -= 1;
        }
        TagEnd::Paragraph => {
            finish_pending_text(state, view);
            state.in_paragraph = false;
            state.paragraph_is_top_level = false;
        }
        TagEnd::Item => {
            finish_pending_text(state, view);
            state.container_depth -= 1;
        }
        TagEnd::List(_) => {
            state.list_depth -= 1;
            state.container_depth -= 1;
        }
        TagEnd::HtmlBlock => {}
        _ => unreachable!("unsupported tag ends are consumed with their rejected starts"),
    }
}

fn append_text(text: &str, state: &mut ParseState) {
    if let Some(cell) = &mut state.current_cell {
        cell.push_str(text);
    } else if state.heading_level.is_some() {
        state.heading_text.push_str(text);
    } else if state.current_table.is_none() {
        state.text_block.push_str(text);
    }
}

fn append_soft_break(state: &mut ParseState) {
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

fn finish_pending_text(state: &mut ParseState, view: &mut DocumentView) {
    let text = state.text_block.trim();
    if !text.is_empty() {
        let text = text.to_owned();
        view.rendered_text.push(text.clone());
        if state.in_paragraph && state.paragraph_is_top_level {
            if let Some(section_index) = state.current_section {
                view.sections[section_index].paragraphs.push(text);
            }
        }
    }
    state.text_block.clear();
}

fn audit_status_marker(view: &DocumentView, violations: &mut Vec<Violation>) -> Option<Status> {
    let markers: Vec<&str> = view
        .html_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| line.starts_with(STATUS_CANDIDATE_PREFIX))
        .collect();

    match markers.as_slice() {
        [] => {
            violations.push(Violation::MissingStatusMarker);
            None
        }
        [line] => {
            let Some(value) = line
                .strip_prefix(STATUS_PREFIX)
                .and_then(|value| value.strip_suffix(STATUS_SUFFIX))
                .filter(|value| {
                    !value.is_empty() && !value.bytes().any(|byte| byte.is_ascii_whitespace())
                })
            else {
                violations.push(Violation::MalformedStatusMarker {
                    line: (*line).to_owned(),
                });
                return None;
            };

            match value {
                "blocked" => Some(Status::Blocked),
                "stopped" => Some(Status::Stopped),
                "executed" => Some(Status::Executed),
                value => {
                    violations.push(Violation::UnknownStatusMarker {
                        value: value.to_owned(),
                    });
                    None
                }
            }
        }
        markers => {
            violations.push(Violation::MultipleStatusMarkers {
                count: markers.len(),
            });
            None
        }
    }
}

fn audit_sections(view: &DocumentView, status: Option<Status>, violations: &mut Vec<Violation>) {
    // pulldown-cmark and the current CommonMark wording diverge around
    // tab-separated closing hashes. Avoid depending on either normalization:
    // only the frozen H2 allowlist may pass, and every other rendered H2 fails.
    for section in &view.sections {
        let heading = section_name(section);
        let is_allowed = ALWAYS_REQUIRED_SECTIONS
            .iter()
            .chain(CONDITIONAL_SECTIONS)
            .any(|allowed| *allowed == heading);
        if !is_allowed {
            violations.push(Violation::UnexpectedSection { heading });
        }
    }

    for &heading in ALWAYS_REQUIRED_SECTIONS
        .iter()
        .chain(CONDITIONAL_SECTIONS)
    {
        let count = view
            .sections
            .iter()
            .filter(|section| section_name(section) == heading)
            .count();
        if count > 1 {
            violations.push(Violation::DuplicateSection(heading));
        }
    }

    for &heading in ALWAYS_REQUIRED_SECTIONS {
        if !has_section(view, heading) {
            violations.push(Violation::MissingSection(heading));
        }
    }

    let Some(status) = status else {
        return;
    };

    let (required, forbidden): (&[&str], &[&str]) = match status {
        Status::Blocked => (
            &["## What was not done", "## Blockers"],
            &["## Why the run was stopped"],
        ),
        Status::Stopped => (
            &["## Why the run was stopped"],
            &["## What was not done", "## Blockers"],
        ),
        Status::Executed => (
            &[],
            &[
                "## What was not done",
                "## Blockers",
                "## Why the run was stopped",
            ],
        ),
    };

    for &heading in required {
        if !has_section(view, heading) {
            violations.push(Violation::MissingSection(heading));
        }
    }
    for &heading in forbidden {
        if has_section(view, heading) {
            violations.push(Violation::ForbiddenSection(heading));
        }
    }
}

fn section_name(section: &Section) -> String {
    format!("## {}", section.heading)
}

fn has_section(view: &DocumentView, heading: &str) -> bool {
    view.sections
        .iter()
        .any(|section| section_name(section) == heading)
}

fn audit_decision(view: &DocumentView, status: Status, violations: &mut Vec<Violation>) {
    let expected_is_in_decision = view
        .sections
        .iter()
        .find(|section| section.heading == "Decision")
        .is_some_and(|section| {
            section
                .paragraphs
                .iter()
                .any(|paragraph| paragraph.trim() == status.decision())
        });
    let another_decision_is_present = [BLOCKED_DECISION, STOPPED_DECISION, EXECUTED_DECISION]
        .into_iter()
        .filter(|decision| *decision != status.decision())
        .any(|decision| {
            view.rendered_text
                .iter()
                .any(|text| text.contains(decision))
        });

    if !expected_is_in_decision || another_decision_is_present {
        violations.push(Violation::DecisionStatusMismatch {
            status: status.as_str(),
        });
    }
}

fn audit_evidence_table(
    view: &DocumentView,
    status: Option<Status>,
    violations: &mut Vec<Violation>,
) {
    let tables = view
        .sections
        .iter()
        .find(|section| section.heading == "Operator evidence template")
        .map(|section| section.tables.as_slice())
        .unwrap_or(&[]);
    if tables.len() != 1 {
        violations.push(Violation::EvidenceTableCount {
            count: tables.len(),
        });
        return;
    }

    let mut counts = vec![0_usize; EVIDENCE_LABELS.len()];
    let mut values = vec![String::new(); EVIDENCE_LABELS.len()];
    let mut encountered_labels = Vec::new();
    let mut has_unexpected_label = false;

    let header_cells = tables[0]
        .iter()
        .find(|row| row.is_header)
        .map(|row| row.cells.clone())
        .unwrap_or_default();
    let expected_header = ["Field", "Value", "Redaction"];
    let header_matches = header_cells.len() == expected_header.len()
        && header_cells
            .iter()
            .zip(expected_header)
            .all(|(actual, expected)| actual == expected);
    if !header_matches {
        violations.push(Violation::EvidenceHeaderMismatch {
            cells: header_cells,
        });
    }

    for row in tables[0].iter().filter(|row| !row.is_header) {
        let label = row.cells.first().map(String::as_str).unwrap_or("");
        if row.cells.len() != 3 {
            violations.push(Violation::EvidenceMalformedRow {
                label: label.to_owned(),
            });
        }
        let Some(label_index) = EVIDENCE_LABELS.iter().position(|expected| *expected == label)
        else {
            has_unexpected_label = true;
            violations.push(Violation::EvidenceUnexpectedLabel {
                label: label.to_owned(),
            });
            continue;
        };

        counts[label_index] += 1;
        encountered_labels.push(label.to_owned());
        values[label_index] = row.cells.get(1).cloned().unwrap_or_default();
    }

    for (index, &label) in EVIDENCE_LABELS.iter().enumerate() {
        match counts[index] {
            0 => violations.push(Violation::EvidenceLabelMissing(label)),
            1 => {
                if let Some(status) = status {
                    audit_evidence_value(label, &values[index], status, violations);
                }
            }
            count => violations.push(Violation::EvidenceLabelDuplicate { label, count }),
        }
    }

    let labels_are_complete = !has_unexpected_label
        && counts.iter().all(|count| *count == 1)
        && encountered_labels.len() == EVIDENCE_LABELS.len();
    if labels_are_complete {
        if let Some((index, found)) = encountered_labels
            .iter()
            .enumerate()
            .find(|(index, found)| found.as_str() != EVIDENCE_LABELS[*index])
        {
            violations.push(Violation::EvidenceOrderMismatch {
                expected: EVIDENCE_LABELS[index],
                found: found.clone(),
            });
        }
    }
}

fn audit_evidence_value(
    label: &'static str,
    value: &str,
    status: Status,
    violations: &mut Vec<Violation>,
) {
    match status {
        Status::Blocked => {
            if value != "UNFILLED" {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::MustBeUnfilled,
                });
            }
        }
        Status::Stopped => {
            if value.is_empty() || value == "UNFILLED" {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::MustBeFilled,
                });
                return;
            }

            if label == "terminal state"
                && !is_not_reached(value)
                && !matches!(
                    value,
                    "STATE_NEW"
                        | "STATE_EXECUTED"
                        | "STATE_MINED"
                        | "STATE_INVALID"
                        | "STATE_FAILED"
                )
            {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::InvalidTerminalState,
                });
            } else if label == "final verdict"
                && !matches!(
                    value,
                    "STOPPED" | "ROLLED_BACK" | "RECONCILIATION_REQUIRED"
                )
            {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::InvalidFinalVerdict,
                });
            }
        }
        Status::Executed => {
            if value.is_empty() || value == "UNFILLED" {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::MustBeFilled,
                });
                return;
            }
            if is_not_reached(value) {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::NotReachedForbidden,
                });
            } else if label == "terminal state" && value != "STATE_CONFIRMED" {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::InvalidTerminalState,
                });
            } else if label == "final verdict"
                && !matches!(
                    value,
                    "CONFIRMED" | "ROLLED_BACK" | "RECONCILIATION_REQUIRED"
                )
            {
                violations.push(Violation::InvalidEvidenceValue {
                    label,
                    rule: EvidenceValueRule::InvalidFinalVerdict,
                });
            }
        }
    }
}

fn is_not_reached(value: &str) -> bool {
    value
        .strip_prefix("NOT_REACHED(")
        .and_then(|reason| reason.strip_suffix(')'))
        .is_some_and(|reason| !reason.is_empty())
}

fn audit_secret_shapes(doc: &str, violations: &mut Vec<Violation>) {
    let bytes = doc.as_bytes();
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
                violations.push(Violation::PrivateKeyLike { run_length });
            } else if run_length >= 40 {
                violations.push(Violation::FullAddress { run_length });
            }
        }
        index += 1;
    }

    if doc.contains("-----BEGIN") {
        violations.push(Violation::PemMarker);
    }
    if doc.contains("Bearer ") {
        violations.push(Violation::BearerMarker);
    }
}

fn decision_document() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join("docs/LIVE_VALIDATION_DECISION.md"))
        .expect("live-validation decision record is readable")
}

fn clean_document() -> String {
    let doc = decision_document();
    let violations = audit(&doc);
    assert!(
        violations.is_empty(),
        "baseline decision record must be clean before mutation: {violations:#?}"
    );
    doc
}

fn replace_exactly_once(source: &str, from: &str, to: &str) -> String {
    assert_eq!(
        source.match_indices(from).count(),
        1,
        "synthetic mutation target must occur exactly once: {from}"
    );
    source.replacen(from, to, 1)
}

fn replace_blocked_status(source: &str, replacement: &str) -> String {
    let from = "<!-- live-validation-status: blocked -->\n\n## Decision";
    let to = format!("{replacement}\n\n## Decision");
    replace_exactly_once(source, from, &to)
}

fn remove_section(source: &str, heading: &str) -> String {
    assert_eq!(
        source.lines().filter(|line| *line == heading).count(),
        1,
        "synthetic section target must occur exactly once: {heading}"
    );
    let start = source.find(heading).unwrap();
    let search_start = start + heading.len();
    let next_heading = source[search_start..]
        .find("\n## ")
        .expect("synthetic section must have a following heading");
    let end = search_start + next_heading + 1;
    format!("{}{}", &source[..start], &source[end..])
}

fn synthetic_row_cells(row: &str) -> Vec<&str> {
    row.trim_matches('|').split('|').map(str::trim).collect()
}

fn set_evidence_value(source: &str, label: &str, value: &str) -> String {
    let matching_rows: Vec<&str> = source
        .lines()
        .filter(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == label)
        })
        .collect();
    assert_eq!(
        matching_rows.len(),
        1,
        "synthetic evidence row must occur exactly once: {label}"
    );
    let row = matching_rows[0];
    let cells = synthetic_row_cells(row);
    assert_eq!(cells.len(), 3, "baseline evidence row must have three cells");
    let replacement = format!("| {} | {} | {} |", cells[0], value, cells[2]);
    replace_exactly_once(source, row, &replacement)
}

fn set_evidence_redaction(source: &str, label: &str, redaction: &str) -> String {
    let matching_rows: Vec<&str> = source
        .lines()
        .filter(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == label)
        })
        .collect();
    assert_eq!(
        matching_rows.len(),
        1,
        "synthetic evidence row must occur exactly once: {label}"
    );
    let row = matching_rows[0];
    let cells = synthetic_row_cells(row);
    assert_eq!(cells.len(), 3, "baseline evidence row must have three cells");
    let replacement = format!("| {} | {} | {} |", cells[0], cells[1], redaction);
    replace_exactly_once(source, row, &replacement)
}

fn remove_evidence_row(source: &str, label: &str) -> String {
    let matching_rows: Vec<&str> = source
        .lines()
        .filter(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == label)
        })
        .collect();
    assert_eq!(matching_rows.len(), 1, "row must occur exactly once: {label}");
    replace_exactly_once(source, &format!("{}\n", matching_rows[0]), "")
}

fn fill_all_evidence_values(mut source: String, status: Status) -> String {
    let values = [
        "approval-ref-redacted",
        "approved-network",
        "tiny-bound",
        "market-redacted",
        "WALLET",
        "fresh-observation-recorded",
        "Live WALLET redacted-scope",
        "bounded-expiry",
        "coarse-submit-time",
        "tx-redacted",
        match status {
            Status::Stopped => "STATE_MINED",
            Status::Executed => "STATE_CONFIRMED",
            Status::Blocked => "UNFILLED",
        },
        match status {
            Status::Stopped => "NOT_REACHED(run stopped before observation)",
            Status::Executed => "redacted-observation",
            Status::Blocked => "UNFILLED",
        },
        "mutation-disabled",
        match status {
            Status::Stopped => "STOPPED",
            Status::Executed => "CONFIRMED",
            Status::Blocked => "UNFILLED",
        },
    ];

    for (&label, value) in EVIDENCE_LABELS.iter().zip(values) {
        source = set_evidence_value(&source, label, value);
    }
    source
}

fn stopped_document() -> String {
    let mut doc = clean_document();
    doc = replace_blocked_status(
        &doc,
        "<!-- live-validation-status: stopped -->",
    );
    doc = replace_exactly_once(&doc, BLOCKED_DECISION, STOPPED_DECISION);
    doc = remove_section(&doc, "## What was not done");
    doc = remove_section(&doc, "## Blockers");
    doc = replace_exactly_once(
        &doc,
        "## What this means for the release",
        "## Why the run was stopped\n\nSynthetic stopped-record reason.\n\n## What this means for the release",
    );
    fill_all_evidence_values(doc, Status::Stopped)
}

fn add_table_inside_list(source: &str) -> String {
    replace_exactly_once(
        source,
        "## What this means for the release",
        "- synthetic evidence table\n\n  | Field | Value | Redaction |\n  | --- | --- | --- |\n  | synthetic field | synthetic value | synthetic redaction |\n\n## What this means for the release",
    )
}

fn operator_evidence_table(source: &str) -> String {
    let heading = "## Operator evidence template\n";
    assert_eq!(source.match_indices(heading).count(), 1);
    let section = &source[source.find(heading).unwrap() + heading.len()..];
    let rows: Vec<&str> = section
        .lines()
        .skip_while(|line| !line.trim().starts_with('|'))
        .take_while(|line| line.trim().starts_with('|') && line.trim().ends_with('|'))
        .collect();
    assert!(!rows.is_empty(), "baseline evidence table exists");
    rows.join("\n")
}

#[test]
fn live_validation_decision_satisfies_offline_contract() {
    let doc = decision_document();
    let violations = audit(&doc);
    assert!(
        violations.is_empty(),
        "live-validation decision audit found violations: {violations:#?}"
    );
}

#[test]
fn blocked_status_with_executed_decision_has_only_decision_mismatch() {
    let doc = clean_document();
    let mutated = replace_exactly_once(&doc, BLOCKED_DECISION, EXECUTED_DECISION);

    assert_eq!(
        audit(&mutated),
        vec![Violation::DecisionStatusMismatch { status: "blocked" }]
    );
}

#[test]
fn executed_supersede_without_decision_update_detects_decision_mismatch() {
    // Mutation A alone does not exercise the full supersede path. This mutation
    // makes status-specific sections and every evidence value valid for
    // `executed`; otherwise rule 2 could reject the document first and let the
    // test pass even if rule 3 were not implemented.
    let mut mutated = clean_document();
    mutated = replace_blocked_status(
        &mutated,
        "<!-- live-validation-status: executed -->",
    );
    mutated = remove_section(&mutated, "## What was not done");
    mutated = remove_section(&mutated, "## Blockers");
    mutated = fill_all_evidence_values(mutated, Status::Executed);

    assert!(audit(&mutated).contains(&Violation::DecisionStatusMismatch {
        status: "executed",
    }));
}

#[test]
fn status_marker_mutations_are_rejected() {
    let doc = clean_document();
    let removed = replace_blocked_status(&doc, "");
    assert_eq!(audit(&removed), vec![Violation::MissingStatusMarker]);

    let mut duplicated = doc.clone();
    duplicated.push_str("\n<!-- live-validation-status: blocked -->\n");
    assert_eq!(
        audit(&duplicated),
        vec![Violation::MultipleStatusMarkers { count: 2 }]
    );

    let unknown = replace_blocked_status(
        &doc,
        "<!-- live-validation-status: pending -->",
    );
    assert_eq!(
        audit(&unknown),
        vec![Violation::UnknownStatusMarker {
            value: "pending".to_owned(),
        }]
    );
}

#[test]
fn blocked_section_mutations_are_rejected() {
    let doc = clean_document();
    let missing_blockers = remove_section(&doc, "## Blockers");
    assert_eq!(
        audit(&missing_blockers),
        vec![Violation::MissingSection("## Blockers")]
    );

    let forbidden_stopped = replace_exactly_once(
        &doc,
        "## What this means for the release",
        "## Why the run was stopped\n\nSynthetic reason.\n\n## What this means for the release",
    );
    assert_eq!(
        audit(&forbidden_stopped),
        vec![Violation::ForbiddenSection(
            "## Why the run was stopped"
        )]
    );
}

#[test]
fn evidence_row_and_blocked_value_mutations_are_rejected() {
    let doc = clean_document();
    let missing_row = remove_evidence_row(&doc, "approved market");
    assert_eq!(
        audit(&missing_row),
        vec![Violation::EvidenceLabelMissing("approved market")]
    );

    let filled = set_evidence_value(&doc, "approved network", "synthetic-network");
    assert_eq!(
        audit(&filled),
        vec![Violation::InvalidEvidenceValue {
            label: "approved network",
            rule: EvidenceValueRule::MustBeUnfilled,
        }]
    );
}

#[test]
fn stopped_value_mutations_are_rejected() {
    let stopped = stopped_document();
    assert!(
        audit(&stopped).is_empty(),
        "synthetic stopped baseline must satisfy every rule"
    );

    let empty = set_evidence_value(&stopped, "approved network", "");
    assert_eq!(
        audit(&empty),
        vec![Violation::InvalidEvidenceValue {
            label: "approved network",
            rule: EvidenceValueRule::MustBeFilled,
        }]
    );

    let confirmed = set_evidence_value(&stopped, "terminal state", "STATE_CONFIRMED");
    assert_eq!(
        audit(&confirmed),
        vec![Violation::InvalidEvidenceValue {
            label: "terminal state",
            rule: EvidenceValueRule::InvalidTerminalState,
        }]
    );
}

#[test]
fn secret_shape_mutations_are_rejected_once_at_the_longest_threshold() {
    let doc = clean_document();

    for (run_length, expected) in [
        (40, Violation::FullAddress { run_length: 40 }),
        (64, Violation::PrivateKeyLike { run_length: 64 }),
        (130, Violation::SignatureLike { run_length: 130 }),
    ] {
        let mut mutated = doc.clone();
        mutated.push_str(&format!("\n0x{}\n", "a".repeat(run_length)));
        assert_eq!(audit(&mutated), vec![expected]);
    }
}

#[test]
fn literal_secret_marker_mutations_are_rejected() {
    let doc = clean_document();

    let mut pem = doc.clone();
    pem.push_str("\n-----BEGIN SYNTHETIC\n");
    assert_eq!(audit(&pem), vec![Violation::PemMarker]);

    let mut bearer = doc;
    bearer.push_str("\nBearer synthetic\n");
    assert_eq!(audit(&bearer), vec![Violation::BearerMarker]);
}

#[test]
fn a_fully_correct_executed_record_passes_every_rule() {
    // Without this, the schema could be strict enough that no legitimate
    // success record exists, and an operator who really confirmed a
    // transaction would have no honest way to write it down.
    let mut doc = clean_document();
    doc = replace_blocked_status(&doc, "<!-- live-validation-status: executed -->");
    doc = replace_exactly_once(&doc, BLOCKED_DECISION, EXECUTED_DECISION);
    doc = remove_section(&doc, "## What was not done");
    doc = remove_section(&doc, "## Blockers");
    doc = fill_all_evidence_values(doc, Status::Executed);

    let violations = audit(&doc);
    assert!(
        violations.is_empty(),
        "a legitimate executed record must pass: {violations:#?}"
    );
}

#[test]
fn a_second_decision_sentence_is_rejected_even_when_the_correct_one_remains() {
    // Rule 3 has two halves: the expected sentence is present, and the other
    // two are absent. Replacing the sentence only exercises the first half, so
    // adding a second status sentence must be rejected on its own.
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        BLOCKED_DECISION,
        &format!("{BLOCKED_DECISION}\n\n{EXECUTED_DECISION}"),
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::DecisionStatusMismatch { status: "blocked" }]
    );
}

#[test]
fn duplicate_and_unexpected_evidence_labels_are_rejected() {
    let doc = clean_document();
    let row = doc
        .lines()
        .find(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == "rollback state")
        })
        .expect("baseline table has a rollback state row")
        .to_owned();

    let duplicated = replace_exactly_once(&doc, &row, &format!("{row}\n{row}"));
    assert_eq!(
        audit(&duplicated),
        vec![Violation::EvidenceLabelDuplicate {
            label: "rollback state",
            count: 2,
        }]
    );

    let unexpected =
        replace_exactly_once(&doc, &row, &format!("{row}\n| bogus label | UNFILLED | none |"));
    assert_eq!(
        audit(&unexpected),
        vec![Violation::EvidenceUnexpectedLabel {
            label: "bogus label".to_owned(),
        }]
    );
}

#[test]
fn not_reached_is_rejected_where_a_real_observation_is_required() {
    let stopped = stopped_document();
    let stopped_without_verdict = set_evidence_value(
        &stopped,
        "final verdict",
        "NOT_REACHED(no verdict)",
    );
    assert_eq!(
        audit(&stopped_without_verdict),
        vec![Violation::InvalidEvidenceValue {
            label: "final verdict",
            rule: EvidenceValueRule::InvalidFinalVerdict,
        }]
    );

    let mut executed = clean_document();
    executed = replace_blocked_status(&executed, "<!-- live-validation-status: executed -->");
    executed = replace_exactly_once(&executed, BLOCKED_DECISION, EXECUTED_DECISION);
    executed = remove_section(&executed, "## What was not done");
    executed = remove_section(&executed, "## Blockers");
    executed = fill_all_evidence_values(executed, Status::Executed);
    let executed_with_gap = set_evidence_value(&executed, "submit time", "NOT_REACHED(skipped)");
    assert_eq!(
        audit(&executed_with_gap),
        vec![Violation::InvalidEvidenceValue {
            label: "submit time",
            rule: EvidenceValueRule::NotReachedForbidden,
        }]
    );
}

#[test]
fn a_secret_shape_inside_a_code_fence_is_still_rejected() {
    // Rules 1-4 use the CommonMark view, which excludes code-block text from
    // normative fields. Rule 5 scans raw source so code cannot hide a secret.
    let mut mutated = clean_document();
    mutated.push_str(&format!("\n```text\n0x{}\n```\n", "b".repeat(64)));

    assert_eq!(
        audit(&mutated),
        vec![Violation::PrivateKeyLike { run_length: 64 }]
    );
}

#[test]
fn stopped_rejects_forbidden_heading_with_two_trailing_spaces() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "## What this means for the release",
        "## What was not done  \n\nNo submit was attempted.\nNo request was sent to a relayer mutation endpoint.\n\n## What this means for the release",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenSection("## What was not done")]
    );
}

#[test]
fn stopped_rejects_forbidden_heading_with_closing_hashes() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "## What this means for the release",
        "## What was not done ##\n\nNo submit was attempted.\nNo request was sent to a relayer mutation endpoint.\n\n## What this means for the release",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenSection("## What was not done")]
    );
}

#[test]
fn stopped_rejects_forbidden_heading_with_two_leading_spaces() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "## What this means for the release",
        "  ## What was not done\n\nNo submit was attempted.\nNo request was sent to a relayer mutation endpoint.\n\n## What this means for the release",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenSection("## What was not done")]
    );
}

#[test]
fn trailing_spaces_cannot_hide_a_second_status_marker() {
    let mut mutated = clean_document();
    mutated.push_str("\n<!-- live-validation-status: executed -->  \n");

    assert_eq!(
        audit(&mutated),
        vec![Violation::MultipleStatusMarkers { count: 2 }]
    );
}

#[test]
fn status_marker_without_required_space_is_malformed() {
    let doc = clean_document();
    let mutated = replace_blocked_status(&doc, "<!-- live-validation-status:blocked -->");

    assert_eq!(
        audit(&mutated),
        vec![Violation::MalformedStatusMarker {
            line: "<!-- live-validation-status:blocked -->".to_owned(),
        }]
    );
}

#[test]
fn decision_appendix_cannot_substitute_for_the_exact_decision_section() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        &format!("## Decision\n\n{BLOCKED_DECISION}"),
        &format!(
            "## Decision appendix\n\n{BLOCKED_DECISION}\n\n## Decision\n\nSynthetic non-canonical decision."
        ),
    );

    assert!(audit(&mutated).contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn evidence_appendix_cannot_hide_a_value_in_the_exact_evidence_section() {
    let doc = clean_document();
    let valid_appendix_table = operator_evidence_table(&doc);
    let contaminated = set_evidence_value(&doc, "approved network", "synthetic");
    let mutated = replace_exactly_once(
        &contaminated,
        "## Operator evidence template",
        &format!(
            "## Operator evidence template appendix\n\n{valid_appendix_table}\n\n## Operator evidence template"
        ),
    );

    assert!(audit(&mutated).contains(&Violation::InvalidEvidenceValue {
        label: "approved network",
        rule: EvidenceValueRule::MustBeUnfilled,
    }));
}

#[test]
fn evidence_rows_must_keep_the_canonical_order() {
    let doc = clean_document();
    let approved_network = doc
        .lines()
        .find(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == "approved network")
        })
        .expect("baseline has approved network row");
    let approved_max_amount = doc
        .lines()
        .find(|line| {
            synthetic_row_cells(line)
                .first()
                .is_some_and(|cell| *cell == "approved max amount")
        })
        .expect("baseline has approved max amount row");
    let mutated = replace_exactly_once(
        &doc,
        &format!("{approved_network}\n{approved_max_amount}"),
        &format!("{approved_max_amount}\n{approved_network}"),
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::EvidenceOrderMismatch {
            expected: "approved network",
            found: "approved max amount".to_owned(),
        }]
    );
}

#[test]
fn evidence_header_must_match_the_canonical_cells() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        "| Field | Value | Redaction |",
        "| Name | Value | Redaction |",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::EvidenceHeaderMismatch {
            cells: vec![
                "Name".to_owned(),
                "Value".to_owned(),
                "Redaction".to_owned(),
            ],
        }]
    );
}

#[test]
fn normalized_duplicate_reserved_sections_are_rejected() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        "## What this means for the release",
        "## Blockers ##\n\nSynthetic duplicate.\n\n## What this means for the release",
    );

    assert!(audit(&mutated).contains(&Violation::DuplicateSection("## Blockers")));
}

#[test]
fn stopped_rejects_forbidden_heading_separated_by_a_tab() {
    // CommonMark accepts a tab after the opening hashes, so `##\tHeading`
    // renders as H2 exactly like `## Heading`. Matching only the space would
    // leave a one-character hole in the same rule the trailing-space case
    // closes.
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\n##\tWhat was not done\n\n- No submit was attempted.",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::ForbiddenSection("## What was not done")]
    );
}

#[test]
fn stopped_rejects_forbidden_heading_with_tab_padded_closing_hashes() {
    // pulldown-cmark 0.13.4 retains a tab-separated closing hash sequence in
    // the rendered heading text. The H2 is still rejected by the allowlist;
    // the rejection class is UnexpectedSection rather than ForbiddenSection.
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\n## What was not done\t##\t\n\n- No submit was attempted.",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::UnexpectedSection {
            heading: "## What was not done\t##".to_owned(),
        }]
    );
}

#[test]
fn indented_fence_cannot_hide_a_forbidden_heading() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\n    ```\n## What was not done\n\nNo request was sent to a relayer mutation endpoint.\n\n    ```",
    );

    assert!(audit(&mutated).contains(&Violation::ForbiddenSection("## What was not done")));
}

#[test]
fn setext_heading_cannot_hide_a_forbidden_section() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\nWhat was not done\n-----------------\n\nNo request was sent to a relayer mutation endpoint.",
    );

    assert!(audit(&mutated).contains(&Violation::ForbiddenSection("## What was not done")));
}

#[test]
fn indented_fence_cannot_hide_a_second_status_marker() {
    let doc = clean_document();
    let mutated = replace_blocked_status(
        &doc,
        "<!-- live-validation-status: blocked -->\n\n    ```\n<!-- live-validation-status: executed -->\n    ```",
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::MultipleStatusMarkers { count: 2 }]
    );
}

#[test]
fn html_comment_heading_does_not_split_the_evidence_section() {
    let doc = clean_document();
    let decoy_table = operator_evidence_table(&doc);
    let contaminated = set_evidence_value(&doc, "approved network", "synthetic");
    let mutated = replace_exactly_once(
        &contaminated,
        "## Operator evidence template",
        &format!(
            "## Operator evidence template\n\n{decoy_table}\n\n<!--\n## audit boundary\n-->"
        ),
    );

    assert_eq!(
        audit(&mutated),
        vec![
            Violation::UnsupportedConstruct {
                kind: "RawHtml".to_owned(),
            },
            Violation::UnsupportedConstruct {
                kind: "RawHtml".to_owned(),
            },
            Violation::UnsupportedConstruct {
                kind: "RawHtml".to_owned(),
            },
            Violation::EvidenceTableCount { count: 2 },
        ]
    );
}

#[test]
fn fenced_marker_examples_are_not_status_candidates() {
    let doc = clean_document();
    let mut construct_violations = Vec::new();
    let view = document_view(&doc, &mut construct_violations);
    assert!(construct_violations.is_empty());
    let marker_candidates: Vec<&str> = view
        .html_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| line.starts_with(STATUS_CANDIDATE_PREFIX))
        .collect();

    assert_eq!(
        marker_candidates,
        vec!["<!-- live-validation-status: blocked -->"]
    );
    let mut violations = Vec::new();
    assert_eq!(
        audit_status_marker(&view, &mut violations),
        Some(Status::Blocked)
    );
    assert!(violations.is_empty());
}

#[test]
fn canonical_decision_inside_code_block_does_not_satisfy_decision_rule() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        &format!("## Decision\n\n{BLOCKED_DECISION}"),
        &format!(
            "## Decision\n\n```text\n{BLOCKED_DECISION}\n```\n\nSynthetic non-canonical decision."
        ),
    );

    assert!(audit(&mutated).contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn inline_html_status_marker_is_rejected() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason. <!-- live-validation-status: executed -->",
    );

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "InlineHtml".to_owned(),
    }));
}

#[test]
fn raw_html_wrapper_around_status_marker_is_rejected() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\n<div><!-- live-validation-status: executed --></div>",
    );

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "RawHtml".to_owned(),
    }));
}

#[test]
fn raw_html_heading_and_prose_are_rejected() {
    let stopped = stopped_document();
    let mutated = replace_exactly_once(
        &stopped,
        "Synthetic stopped-record reason.",
        "Synthetic stopped-record reason.\n\n<h2>What was not done</h2>\n<p>No mutation request was sent.</p>",
    );

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "RawHtml".to_owned(),
    }));
}

#[test]
fn raw_html_table_before_evidence_table_is_rejected() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        "## Operator evidence template",
        "## Operator evidence template\n\n<table><tr><th>Field</th><th>Value</th><th>Redaction</th></tr></table>",
    );

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "RawHtml".to_owned(),
    }));
}

#[test]
fn image_alt_text_cannot_satisfy_the_decision_rule() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        BLOCKED_DECISION,
        "![Live validation was not performed.](contradictory-evidence.svg)",
    );
    let violations = audit(&mutated);

    assert!(violations.contains(&Violation::UnsupportedConstruct {
        kind: "Image".to_owned(),
    }));
    assert!(violations.contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn image_alt_text_cannot_supply_an_evidence_value() {
    let doc = clean_document();
    let mutated = set_evidence_value(
        &doc,
        "approved network",
        "![UNFILLED](production-network.svg)",
    );
    let violations = audit(&mutated);

    assert!(violations.contains(&Violation::UnsupportedConstruct {
        kind: "Image".to_owned(),
    }));
    assert!(violations.contains(&Violation::InvalidEvidenceValue {
        label: "approved network",
        rule: EvidenceValueRule::MustBeUnfilled,
    }));
}

#[test]
fn a_middle_h1_ends_the_decision_section_and_is_rejected() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        BLOCKED_DECISION,
        "# Archived lifecycle statement\n\nLive validation was not performed.",
    );
    let violations = audit(&mutated);

    assert!(violations.contains(&Violation::UnsupportedConstruct {
        kind: "Heading1".to_owned(),
    }));
    assert!(violations.contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn a_nested_list_cannot_reassemble_the_decision_sentence() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        BLOCKED_DECISION,
        "- Live validation was not per\n  - formed.",
    );
    let violations = audit(&mutated);

    assert!(violations.contains(&Violation::UnsupportedConstruct {
        kind: "NestedList".to_owned(),
    }));
    assert!(violations.contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn another_status_sentence_in_a_redaction_cell_is_rejected() {
    let doc = clean_document();
    let mutated = set_evidence_redaction(
        &doc,
        "operator approval reference",
        EXECUTED_DECISION,
    );

    assert_eq!(
        audit(&mutated),
        vec![Violation::DecisionStatusMismatch { status: "blocked" }]
    );
}

#[test]
fn a_deep_heading_cannot_hide_another_status_sentence() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        "## What this means for the release",
        &format!(
            "### {EXECUTED_DECISION}\n\n## What this means for the release"
        ),
    );
    let violations = audit(&mutated);

    assert!(violations.contains(&Violation::UnsupportedConstruct {
        kind: "HeadingDepth".to_owned(),
    }));
    assert!(violations.contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn a_decision_heading_inside_a_list_is_rejected() {
    let doc = clean_document();
    let mutated = replace_exactly_once(&doc, "## Decision\n", "- ## Decision\n");

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "HeadingInContainer".to_owned(),
    }));
}

#[test]
fn a_decision_list_item_is_not_a_decision_paragraph() {
    let doc = clean_document();
    let mutated = replace_exactly_once(&doc, BLOCKED_DECISION, "- Live validation was not performed.");

    assert!(audit(&mutated).contains(&Violation::DecisionStatusMismatch {
        status: "blocked",
    }));
}

#[test]
fn an_evidence_heading_inside_a_list_is_rejected() {
    let doc = clean_document();
    let mutated = replace_exactly_once(
        &doc,
        "## Operator evidence template\n",
        "- ## Operator evidence template\n",
    );

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "HeadingInContainer".to_owned(),
    }));
}

#[test]
fn a_table_inside_a_list_is_rejected() {
    let doc = clean_document();
    let mutated = add_table_inside_list(&doc);

    assert!(audit(&mutated).contains(&Violation::UnsupportedConstruct {
        kind: "TableInContainer".to_owned(),
    }));
}

/// Regression net for every bypass found during the implementation-gate
/// rounds. Each entry once made `audit()` return an empty `Vec` on a document
/// that a reader would see as violating the contract. They are kept together
/// so that a later change to the audit cannot quietly reopen an old one.
#[test]
fn every_bypass_found_during_review_stays_closed() {
    let doc = clean_document();
    let stopped = stopped_document();
    let row = doc.lines().find(|l| l.starts_with("| operator approval reference |")).unwrap().to_owned();
    let mut open = Vec::new();

    let mut decoy_tbl = String::from("## Operator evidence template\n\n| Field | Value | Redaction |\n| --- | --- | --- |\n");
    for label in EVIDENCE_LABELS { decoy_tbl.push_str(&format!("| {label} | UNFILLED | decoy |\n")); }
    decoy_tbl.push_str("\n<!--\n## audit boundary\n-->\n");
    let corrupted = set_evidence_value(&doc, "approved network", "synthetic-leak");

    let cases: Vec<(&str, String)> = vec![
      ("R1a trailing-space head", replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n## What was not done  \n\n- No request was sent.")),
      ("R1b closing-hash head",   replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n## What was not done ##\n\n- No request was sent.")),
      ("R1c leading-space head",  replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n  ## What was not done\n\n- No request was sent.")),
      ("R2a tab-separated head",  replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n##\tWhat was not done\n\n- No request was sent.")),
      ("R2b tab-padded closing",  replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n## What was not done\t##\t\n\n- No request was sent.")),
      ("R3a indented fence",      replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n    ```\n## What was not done\n\nNo request was sent.\n\n    ```\n")),
      ("R3b setext heading",      replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\nWhat was not done\n-----------------\n\nNo request was sent.\n")),
      ("R3c html-comment split",  replace_exactly_once(&corrupted, "## Operator evidence template", &decoy_tbl)),
      ("R4a inline marker",       replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason. <!-- live-validation-status: executed -->")),
      ("R4b wrapped marker",      replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n<div><!-- live-validation-status: executed --></div>\n")),
      ("R4c raw html h2",         replace_exactly_once(&stopped, "Synthetic stopped-record reason.", "Synthetic stopped-record reason.\n\n<h2>What was not done</h2>\n<p>No request was sent.</p>\n")),
      ("R4d raw html table",      replace_exactly_once(&doc, "| Field | Value | Redaction |", "<table><tr><th>Field</th></tr></table>\n\n| Field | Value | Redaction |")),
      ("R4e image alt decision",  replace_exactly_once(&doc, BLOCKED_DECISION, "![Live validation was not performed.](x.svg)")),
      ("R4f image alt value",     set_evidence_value(&doc, "approved network", "![UNFILLED](x.svg)")),
      ("R5a mid-document h1",     replace_exactly_once(&doc, BLOCKED_DECISION, "# Archived lifecycle statement\n\nLive validation was not performed.")),
      ("R5b nested list splice",  replace_exactly_once(&doc, BLOCKED_DECISION, "- Live validation was not per\n  - formed.")),
      ("R5c sentence in cell",    replace_exactly_once(&doc, &row, "| operator approval reference | UNFILLED | Live validation reached a confirmed transaction state. |")),
      ("R5d sentence in h3",      replace_exactly_once(&doc, "## What this means for the release", "### Live validation reached a confirmed transaction state.\n\n## What this means for the release")),
      ("R6a decision h2 in list", replace_exactly_once(&doc, "## Decision\n", "- ## Decision\n")),
      ("R6b decision list item",  replace_exactly_once(&doc, BLOCKED_DECISION, "- Live validation was not performed.")),
      ("R6c evidence h2 in list", replace_exactly_once(&doc, "## Operator evidence template\n", "- ## Operator evidence template\n")),
      ("R6d table in list",       add_table_inside_list(&doc)),
    ];

    for (name, mutated) in &cases {
        if audit(mutated).is_empty() {
            open.push(*name);
        }
    }
    assert!(
        open.is_empty(),
        "these bypasses were found during review and must stay closed: {open:?}"
    );
}
