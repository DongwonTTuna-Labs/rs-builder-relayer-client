from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOW = ROOT / ".github" / "workflows" / "codex-review-orchestrator.yml"


def load_workflow():
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def test_single_orchestrator_workflow_exists():
    workflows = list((ROOT / ".github" / "workflows").glob("*.yml")) + list((ROOT / ".github" / "workflows").glob("*.yaml"))
    assert [p.name for p in workflows] == ["codex-review-orchestrator.yml"]


def test_workflow_declares_expected_stage_order():
    jobs = list(load_workflow()["jobs"].keys())
    expected = [
        "bootstrap_event",
        "resolve_collect",
        "resolve_triage_model",
        "resolve_apply_trusted",
        "review_axes_model",
        "review_combine",
        "techlead_model",
        "review_publish_trusted",
        "design_context",
        "design_model_chain",
        "design_chief_model",
        "design_publish_trusted",
        "fix_dispatch_and_merge",
        "push_validate_no_token",
        "push_trusted",
        "record_reentry",
    ]
    assert jobs == expected


def test_no_inline_python_or_schema_bloat():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "python - <<" not in text
    assert "json-schema.org" not in text
    assert text.count("setup/codex-review/bin/codex-review") >= 8


def test_no_placeholder_echo_json_or_error_suppression():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "|| true" not in text
    assert "echo '{\"schema_version\"" not in text
    assert " default-result " not in text
    assert " default-" not in text
    assert "model-result" in text
    assert "run-agents" in text
    assert "model-merged-fix" in text
    assert "stage07 validate-fix" in text
    assert "stage07 commit-push" in text
    assert "stage07 push" in text
    assert "stage08 validate" in text


def test_workflow_routes_design_and_fix_stages():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "needs.resolve_apply_trusted.outputs.route == 'run_design_from_existing_threads'" in text
    assert "needs.review_publish_trusted.outputs.route == 'run_design'" in text
    assert "needs.design_publish_trusted.outputs.route == 'run_stage05'" in text


def test_actions_are_pinned_and_checkout_credentials_not_persisted():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "actions/checkout@v4" not in text
    assert "actions/upload-artifact@v4" not in text
    assert "actions/download-artifact@v4" not in text
    assert text.count("actions/checkout@08eba0b27e820071cde6df949e0beb9ba4906955") == text.count("persist-credentials: false")


def test_stage03_plan_is_validated_in_workflow():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "stage03 model-plan" in text
    assert "stage03 validate-plan" in text
    assert "design-plan.raw.json" in text


def test_workflow_dispatch_pr_number_is_threaded_into_context():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "github.event.inputs.pr_number" in text
    assert "CODEX_REVIEW_PR_NUMBER" in text


def test_default_model_command_uses_oidc_codex_runner():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "CODEX_REVIEW_MODEL_COMMAND: ${{ vars.CODEX_REVIEW_MODEL_COMMAND || 'setup/codex-review/bin/codex-review-model-runner' }}" in text
    assert (ROOT / "setup" / "codex-review" / "bin" / "codex-review-model-runner").is_file()


def test_fix_and_stage07_use_pr_head_worktree():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "path: pr-head" in text
    assert "stage05 run-agents" in text and "--repo-path pr-head" in text
    assert "stage06 premerge" in text and "--repo-path pr-head" in text
    assert "stage07 validate-fix" in text and "--repo-path pr-head" in text


def test_no_token_validation_job_does_not_request_app_token():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("push_validate_no_token:", 1)[1].split("push_trusted:", 1)[0]
    assert "auth app-token" not in section
    assert "GITHUB_TOKEN:" not in section
    assert "stage07 validate-fix" in section


def test_workflow_installs_helper_dependencies_and_pins_python_runtime():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405" in text
    assert "python-version: '3.11'" in text
    assert "pip install --disable-pip-version-check -e setup/codex-review" in text
    assert "pip install --disable-pip-version-check -e trusted/setup/codex-review" in text


def test_autofix_path_is_same_repo_and_pr_head_checkout_is_explicit():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "github.event.pull_request.head.repo.full_name == github.event.pull_request.base.repo.full_name" in text
    assert "repository: ${{ github.event.pull_request.head.repo.full_name || github.repository }}" in text


def test_fix_model_commands_run_from_trusted_checkout_not_pr_head():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("fix_dispatch_and_merge:", 1)[1].split("push_validate_no_token:", 1)[0]
    assert "CODEX_REVIEW_MODEL_CWD: ${{ github.workspace }}/trusted" in section
    assert "CODEX_REVIEW_TARGET_REPO_PATH: ${{ github.workspace }}/pr-head" in section
