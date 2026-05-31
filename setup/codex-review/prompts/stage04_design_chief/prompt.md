Approve or reject the design. Return codex.stage04.model_design_chief.v1 JSON only.
The PR head checkout is in workspace; inspect review target files and diffs through that path.
Approve exact test_plan commands; reject markdown, backticks, prose, or combined command strings.
Stage06 separates Stage07 push-safe validation_commands from full deferred_validation_commands, so do not reject exact cargo test, cargo clippy, or python3 unittest commands solely because Stage07 will not run them.
