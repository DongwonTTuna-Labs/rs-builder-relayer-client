# Codex Review Orchestrator Implementation Order

1. Keep workflow structure tests red until the new workflow, helper root, prompt files, and schema files exist.
2. Move existing Python contracts into `setup/codex-review/src/codex_review` without changing artifact schema versions or stage command names.
3. Split stage implementations under `src/codex_review/stages/stageXX_name`.
4. Extract model prompts into `prompts/` and model output schemas into `schemas/`.
5. Cut workflow invocations over to `setup/codex-review/bin/codex-review`.
6. Remove the legacy helper root and legacy workflow.
7. Run local Python, workflow/static, Rust, GitHub E2E, and visible ChatGPT Pro review.
