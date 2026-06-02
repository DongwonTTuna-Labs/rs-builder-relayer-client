from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOW = ROOT / ".github" / "workflows" / "codex-review-orchestrator.yml"
CODEX_ACTION = "openai/codex-action@e0fdf01220eb9a88167c4898839d273e3f2609d1"


def load_workflow():
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def iter_job_steps():
    for job_name, job in load_workflow()["jobs"].items():
        for step in job.get("steps", []):
            yield job_name, step


def codex_action_steps():
    return [
        (job_name, step)
        for job_name, step in iter_job_steps()
        if step.get("uses") == CODEX_ACTION
    ]


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
        "fix_prepare",
        "fix_agent_model",
        "fix_collect_and_merge",
        "semantic_patch_safety_model",
        "push_validate_no_token",
        "push_trusted",
        "record_reentry",
        "issue_fallback_trusted",
    ]
    assert jobs == expected


def test_workflow_cancels_stale_runs_for_same_pr():
    workflow = load_workflow()
    assert workflow["concurrency"]["group"] == "codex-review-v3-${{ github.event.pull_request.number || github.event.inputs.pr_number || github.run_id }}"
    assert workflow["concurrency"]["cancel-in-progress"] is True


def test_no_inline_python_or_schema_bloat():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "python - <<" not in text
    assert "json-schema.org" not in text
    assert text.count("workflow-helper/setup/codex-review/bin/codex-review") >= 8


def test_no_placeholder_echo_json_or_error_suppression():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "|| true" not in text
    assert "echo '{\"schema_version\"" not in text
    assert " default-result " not in text
    assert " default-" not in text
    assert "model-result" not in text
    assert "run-agents" not in text
    assert "model-merged-fix" not in text
    assert "stage06 build-semantic-safety-prompt" in text
    assert "stage06 validate-semantic-safety" in text
    assert "stage07 validate-fix" in text
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in text
    assert "stage07 commit-push" in text
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
    assert "stage03 build-plan-prompt" in text
    assert "stage03 validate-plan" in text
    assert "design-plan.raw.json" in text


def test_workflow_dispatch_pr_number_is_threaded_into_context():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "github.event.inputs.pr_number" in text
    assert "CODEX_REVIEW_PR_NUMBER" in text


def test_bootstrap_collects_openspec_context_artifacts():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("bootstrap_event:", 1)[1].split("resolve_collect:", 1)[0]
    assert "context openspec --pr-context codex-review-artifacts/event/pr-context.json" in section
    assert "context openspec-markdown --in codex-review-artifacts/event/openspec-context.json" in section
    assert "openspec-context.json" in section
    assert "openspec-context.md" in section
    assert "docs-context.md" in section and "openspec-context.md >> codex-review-artifacts/event/docs-context.md" in section


def test_workflow_uses_codex_action_for_model_execution():
    steps = codex_action_steps()
    assert len(steps) >= 10
    for job_name, step in steps:
        with_inputs = step["with"]
        assert "openai-api-key" not in with_inputs, job_name
        assert "responses-api-endpoint" not in with_inputs, job_name
        assert with_inputs["codex-args"] == "${{ steps.relay-token.outputs.codex_args }}", job_name
        assert step["env"]["AI_RELAY_API_KEY"] == "${{ steps.relay-token.outputs.relay_token }}", job_name
        assert with_inputs["sandbox"] == "read-only", job_name
        assert with_inputs["safety-strategy"] == "read-only", job_name
        assert with_inputs["allow-users"] == "DongwonTTuna", job_name
        assert with_inputs["allow-bots"] is True, job_name
        assert with_inputs["allow-bot-users"] == "codex-reviewer-for-dongwonttuna[bot]", job_name
        assert with_inputs["prompt-file"], job_name
        assert with_inputs["output-file"], job_name
        assert with_inputs["output-schema-file"].endswith(".openai.schema.json"), job_name
        assert "codex-review-artifacts/schemas/" in with_inputs["output-schema-file"], job_name
        assert with_inputs["working-directory"], job_name


def test_stage01_to_stage04_model_jobs_use_pr_head_worktree():
    jobs = load_workflow()["jobs"]
    stage_jobs = [
        "review_axes_model",
        "techlead_model",
        "design_context",
        "design_model_chain",
        "design_chief_model",
    ]
    for job_name in stage_jobs:
        job = jobs[job_name]
        checkout_steps = [
            step
            for step in job.get("steps", [])
            if step.get("uses", "").startswith("actions/checkout@")
            and (step.get("with") or {}).get("path") == "pr-head"
        ]
        assert checkout_steps, job_name
        head_checkout = checkout_steps[0]["with"]
        assert head_checkout["repository"] == "${{ needs.bootstrap_event.outputs.head_repo_full_name || github.repository }}"
        assert head_checkout["ref"] == "${{ needs.bootstrap_event.outputs.head_sha || github.sha }}"
        assert head_checkout["persist-credentials"] is False
        for step in job.get("steps", []):
            if step.get("uses") == CODEX_ACTION:
                with_inputs = step["with"]
                assert with_inputs["working-directory"] == "${{ github.workspace }}/pr-head", job_name
                assert with_inputs["prompt-file"].startswith("${{ github.workspace }}/"), job_name
                assert with_inputs["output-file"].startswith("${{ github.workspace }}/"), job_name
                assert with_inputs["output-schema-file"].startswith("${{ github.workspace }}/"), job_name


def test_stage01_to_stage04_validators_receive_pr_head_repo_path():
    text = WORKFLOW.read_text(encoding="utf-8")
    for command in [
        "stage01 validate",
        "stage02 validate",
        "stage03 validate-plan",
        "stage04 validate",
    ]:
        assert command in text
    for snippet in [
        "stage01 validate --axis ${{ matrix.axis }}",
        "stage02 validate --inventory",
        "stage03 validate-plan --in",
        "stage04 validate --in",
    ]:
        start = text.index(snippet)
        line = text[start:text.index("\n", start)]
        assert "--repo-path pr-head" in line, snippet


def test_codex_action_reuses_relay_home_for_rootless_server_info_placeholder():
    jobs = load_workflow()["jobs"]
    for job_name, job in jobs.items():
        relay_steps = [
            step
            for step in job.get("steps", [])
            if step.get("uses") == "DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main"
        ]
        action_steps = [step for step in job.get("steps", []) if step.get("uses") == CODEX_ACTION]
        if not action_steps:
            continue
        assert len(relay_steps) == 1, job_name
        relay_home = relay_steps[0]["with"]["codex-home"]
        for step in action_steps:
            assert step["with"]["codex-home"] == relay_home, job_name


def test_workflow_generates_openai_strict_schemas_for_codex_action():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert text.count("schema openai-strict --schema") >= len(codex_action_steps())
    for _, step in codex_action_steps():
        schema_file = step["with"]["output-schema-file"]
        schema_name = Path(schema_file).name.removesuffix(".openai.schema.json")
        assert f"schema openai-strict --schema {schema_name}" in text


def test_workflow_has_no_model_runner_default_or_codex_cli_env_contract():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "CODEX_REVIEW_MODEL_COMMAND" not in text
    assert "CODEX_REVIEW_CODEX_ARGS_JSON" not in text
    assert "codex-review-model-runner" not in text


def test_fix_and_stage07_use_pr_head_worktree():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "path: pr-head" in text
    assert "stage05 prepare-agents" in text and "--repo-path pr-head" in text
    assert "stage06 premerge" in text and "--repo-path pr-head" in text
    assert "stage06 build-semantic-safety-prompt" in text and "--repo-path pr-head" in text
    assert "stage07 validate-fix" in text and "--repo-path pr-head" in text


def test_no_token_validation_job_does_not_request_app_token():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("push_validate_no_token:", 1)[1].split("push_trusted:", 1)[0]
    assert "auth app-token" not in section
    assert "GITHUB_TOKEN:" not in section
    assert "stage07 validate-fix" in section
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in section
    assert "stage02 write-deferred-outputs" not in section


def test_semantic_patch_safety_model_gates_stage07_push_validation():
    text = WORKFLOW.read_text(encoding="utf-8")
    jobs = load_workflow()["jobs"]
    assert "semantic_patch_safety_model" in jobs
    semantic = jobs["semantic_patch_safety_model"]
    assert semantic["needs"] == ["bootstrap_event", "fix_collect_and_merge"]
    semantic_text = text.split("semantic_patch_safety_model:", 1)[1].split("push_validate_no_token:", 1)[0]
    assert "stage06 build-semantic-safety-prompt" in semantic_text
    assert "schema openai-strict --schema stage06-semantic-patch-safety.v1" in semantic_text
    assert "stage06 validate-semantic-safety" in semantic_text
    assert "stage06 write-semantic-safety-outputs" in semantic_text
    assert "AI_RELAY_API_KEY" in semantic_text
    validate_job = jobs["push_validate_no_token"]
    assert validate_job["needs"] == ["bootstrap_event", "fix_collect_and_merge", "semantic_patch_safety_model"]


def test_push_and_issue_fallback_are_default_actual_write_paths():
    text = WORKFLOW.read_text(encoding="utf-8")
    push_flag = "CODEX_REVIEW" + "_ENABLE" + "_PUSH"
    issue_flag = "CODEX_REVIEW" + "_ENABLE" + "_ISSUE_FALLBACK"
    assert push_flag not in text
    assert issue_flag not in text

    assert "stage02 write-deferred-outputs" in text

    validate_section = text.split("push_validate_no_token:", 1)[1].split("push_trusted:", 1)[0]
    assert "stage07 validate-fix --dry-run" not in validate_section
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in validate_section
    assert "stage07 write-validation-outputs" in validate_section
    assert "requires_push_token" in validate_section

    push_section = text.split("push_trusted:", 1)[1].split("record_reentry:", 1)[0]
    assert "if: needs.push_validate_no_token.outputs.requires_push_token == 'true'" in push_section
    assert "auth app-token --mode push" in push_section
    assert "stage07 commit-push --in" in push_section
    assert "stage07 push --dry-run" not in push_section

    record_section = text.split("record_reentry:", 1)[1].split("issue_fallback_trusted:", 1)[0]
    assert "if: needs.push_trusted.outputs.push_status == 'pushed'" in record_section
    assert "auth app-token --mode stage08" in record_section
    assert "stage08 record --in" in record_section
    assert "--pr-context codex-review-artifacts/event/pr-context.json" in record_section

    issue_section = text.split("issue_fallback_trusted:", 1)[1]
    assert "auth app-token --mode stage09" in issue_section
    assert "stage09 apply --in" in issue_section
    assert "stage09 apply --dry-run" not in issue_section


def test_workflow_installs_helper_dependencies_and_pins_python_runtime():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405" in text
    assert "python-version: '3.11'" in text
    assert "pip install --disable-pip-version-check -e workflow-helper/setup/codex-review" in text
    assert "pip install --disable-pip-version-check -e trusted/setup/codex-review" not in text


def test_workflow_helper_checkout_uses_workflow_sha_without_changing_base_ref():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "ref: ${{ github.event.pull_request.base.sha || github.sha }}" in text
    assert "repository: ${{ github.repository }}" in text
    assert "ref: ${{ github.workflow_sha }}" in text
    assert "path: workflow-helper" in text

    jobs = load_workflow()["jobs"]
    for job_name, job in jobs.items():
        helper_steps = [
            step
            for step in job.get("steps", [])
            if step.get("name") == "Checkout workflow helper"
        ]
        assert len(helper_steps) == 1, job_name
        helper = helper_steps[0]
        assert helper["uses"] == "actions/checkout@08eba0b27e820071cde6df949e0beb9ba4906955"
        assert helper["with"]["repository"] == "${{ github.repository }}"
        assert helper["with"]["ref"] == "${{ github.workflow_sha }}"
        assert helper["with"]["path"] == "workflow-helper"
        assert helper["with"]["persist-credentials"] is False


def test_setup_python_pip_cache_uses_workflow_helper_dependency_file():
    setup_steps = [
        (job_name, step)
        for job_name, step in iter_job_steps()
        if step.get("uses") == "actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405"
    ]
    assert setup_steps
    for job_name, step in setup_steps:
        assert step["with"]["python-version"] == "3.11", job_name
        assert step["with"]["cache"] == "pip", job_name
        assert step["with"]["cache-dependency-path"] == "workflow-helper/setup/codex-review/pyproject.toml", job_name


def test_workflow_never_executes_helper_from_pr_head_or_stale_trusted_tree():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "pr-head/setup/codex-review" not in text
    assert "trusted/setup/codex-review" not in text


def test_autofix_path_is_same_repo_and_pr_head_checkout_is_explicit():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("fix_prepare:", 1)[1].split("fix_agent_model:", 1)[0]
    assert "github.event_name == 'pull_request_target'" not in section
    assert "needs.bootstrap_event.outputs.same_repo == 'true'" in section
    assert "repository: ${{ needs.bootstrap_event.outputs.head_repo_full_name || github.repository }}" in text
    assert "ref: ${{ needs.bootstrap_event.outputs.head_sha || github.sha }}" in text



def test_stage09_issue_fallback_uses_app_token_and_never_github_token_write():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "issue_fallback_trusted:" in text
    section = text.split("issue_fallback_trusted:", 1)[1]
    assert "auth app-token --mode stage09" in section
    issue_flag = "CODEX_REVIEW" + "_ENABLE" + "_ISSUE_FALLBACK"
    assert issue_flag not in section
    assert "--dry-run" not in section
    assert "stage09 plan" in section
    assert "stage09 apply" in section
    assert "has_deferred_issue_items" in section
    assert "stage02_defer_to_issue" in section
    assert "codex-review-stage02" in section
    for status in ["no_diff_repeat", "empty_patch", "tests_failed", "validation_failed", "committed_no_head_ref"]:
        assert f"push_status == '{status}'" in section
    assert "issues: write" not in section


def test_fix_model_commands_run_from_trusted_checkout_not_pr_head():
    text = WORKFLOW.read_text(encoding="utf-8")
    section = text.split("fix_agent_model:", 1)[1].split("fix_collect_and_merge:", 1)[0]
    assert "working-directory: ${{ github.workspace }}/pr-head" in section
    assert "CODEX_REVIEW_MODEL_CWD" not in section
    assert "CODEX_REVIEW_TARGET_REPO_PATH" not in section
