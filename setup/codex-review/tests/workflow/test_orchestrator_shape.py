from pathlib import Path
import yaml

from _pipeline import all_jobs as pipeline_jobs
from _pipeline import all_text as pipeline_text
from _pipeline import codex_action_steps as pipeline_codex_action_steps
from _pipeline import iter_all_steps

ROOT = Path(__file__).resolve().parents[4]
REVIEW = ROOT / ".github" / "workflows" / "codex-review.yml"
DESIGN = ROOT / ".github" / "workflows" / "codex-design.yml"
FIX = ROOT / ".github" / "workflows" / "codex-fix.yml"
ISSUE = ROOT / ".github" / "workflows" / "codex-issue.yml"
CODEX_ACTION = "openai/codex-action@e0fdf01220eb9a88167c4898839d273e3f2609d1"


def load_review():
    return yaml.safe_load(REVIEW.read_text(encoding="utf-8"))


def load_design():
    return yaml.safe_load(DESIGN.read_text(encoding="utf-8"))


def load_fix():
    return yaml.safe_load(FIX.read_text(encoding="utf-8"))


def load_issue():
    return yaml.safe_load(ISSUE.read_text(encoding="utf-8"))


def test_codex_pipeline_workflow_files_present():
    workflows = list((ROOT / ".github" / "workflows").glob("*.yml")) + list((ROOT / ".github" / "workflows").glob("*.yaml"))
    names = {p.name for p in workflows}
    # The monolithic orchestrator has been fully replaced by four label-driven workflows.
    assert names == {
        "codex-review.yml",
        "codex-design.yml",
        "codex-fix.yml",
        "codex-issue.yml",
    }


def test_workflow_declares_expected_stage_order():
    review_jobs = list(load_review()["jobs"].keys())
    assert review_jobs == [
        "guard",
        "collect_threads",
        "triage_threads",
        "apply_threads",
        "review_axes",
        "combine_findings",
        "techlead",
        "publish_review",
        "finalize",
    ]
    design_jobs = list(load_design()["jobs"].keys())
    assert design_jobs == [
        "guard",
        "design_context",
        "prepare_clusters",
        "analyze_clusters",
        "draft_plan",
        "chief_decision",
        "publish_design",
        "finalize",
    ]
    fix_jobs = list(load_fix()["jobs"].keys())
    assert fix_jobs == [
        "guard",
        "plan_tasks",
        "run_agents",
        "merge_fixes",
        "patch_safety",
        "validate_patch",
        "commit_push",
        "finalize",
    ]
    issue_jobs = list(load_issue()["jobs"].keys())
    assert issue_jobs == ["compose_content", "publish_issue"]


def test_each_stage_workflow_serializes_per_pr_without_cancelling():
    # Each split workflow runs at most one instance per PR and never cancels an
    # in-flight run (the loop relies on label transitions completing).
    for loader, group_prefix in [
        (load_review, "codex-review-"),
        (load_design, "codex-design-"),
        (load_fix, "codex-fix-"),
        (load_issue, "codex-issue-"),
    ]:
        workflow = loader()
        assert workflow["concurrency"]["group"] == group_prefix + "${{ github.event.pull_request.number }}"
        assert workflow["concurrency"]["cancel-in-progress"] is False


def test_no_inline_python_or_schema_bloat():
    text = pipeline_text()
    assert "python - <<" not in text
    assert "json-schema.org" not in text
    assert text.count("workflow-helper/setup/codex-review/bin/codex-review") >= 8


def test_no_placeholder_echo_json_or_error_suppression():
    # Helper/model commands must not swallow errors or emit placeholder JSON.
    # (Best-effort `|| true` on gh label ops in guard/finalize is intentional and
    #  scoped to label mutations, so this invariant targets helper/model commands.)
    fix_text = FIX.read_text(encoding="utf-8")
    assert "echo '{\"schema_version\"" not in fix_text
    assert " default-result " not in fix_text
    assert " default-" not in fix_text
    assert "model-result" not in fix_text
    assert "run-agents" not in fix_text
    assert "model-merged-fix" not in fix_text
    assert "stage08 validate" not in fix_text
    # The semantic-safety + push commands now live in the fix workflow.
    assert "stage06 build-semantic-safety-prompt" in fix_text
    assert "stage06 validate-semantic-safety" in fix_text
    assert "stage07 validate-fix" in fix_text
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in fix_text
    assert "stage07 commit-push" in fix_text


def test_workflow_routes_design_and_fix_stages():
    # Review decides whether to design via its finalize label transition;
    # design gates the fix loop on the chief's route; fix loops back to review.
    assert "run_design" in REVIEW.read_text(encoding="utf-8")
    assert "리뷰완료" in REVIEW.read_text(encoding="utf-8")
    design_text = DESIGN.read_text(encoding="utf-8")
    assert 'DESIGN_ROUTE" = "run_stage05"' in design_text
    assert "설계완료" in design_text
    assert "리뷰중" in FIX.read_text(encoding="utf-8")


def test_actions_are_pinned_and_checkout_credentials_not_persisted():
    text = pipeline_text()
    assert "actions/checkout@v4" not in text
    assert "actions/upload-artifact@v4" not in text
    assert "actions/download-artifact@v4" not in text
    assert text.count("actions/checkout@08eba0b27e820071cde6df949e0beb9ba4906955") == text.count("persist-credentials: false")


def test_stage03_plan_is_validated_in_workflow():
    text = DESIGN.read_text(encoding="utf-8")
    assert "stage03 build-plan-prompt" in text
    assert "stage03 validate-plan" in text
    assert "design-plan.raw.json" in text


def test_label_triggered_workflows_thread_pr_number_into_context():
    # Split workflows are label-driven (no workflow_dispatch); each threads the
    # PR number from the labeled event into the helper env.
    for path in (REVIEW, DESIGN, FIX, ISSUE):
        text = path.read_text(encoding="utf-8")
        assert "types: [labeled]" in text
        assert "CODEX_REVIEW_PR_NUMBER: ${{ github.event.pull_request.number }}" in text


def test_bootstrap_collects_openspec_context_artifacts():
    text = REVIEW.read_text(encoding="utf-8")
    section = text.split("  guard:", 1)[1].split("collect_threads:", 1)[0]
    assert "context openspec --pr-context codex-review-artifacts/event/pr-context.json" in section
    assert "context openspec-markdown --in codex-review-artifacts/event/openspec-context.json" in section
    assert "openspec-context.json" in section
    assert "openspec-context.md" in section
    assert "docs-context.md" in section and "openspec-context.md >> codex-review-artifacts/event/docs-context.md" in section


def test_workflow_uses_codex_action_for_model_execution():
    steps = pipeline_codex_action_steps()
    assert len(steps) >= 10
    for job_name, step in steps:
        with_inputs = step["with"]
        # Native codex-action relay wiring: the OIDC-minted key drives the
        # built-in Responses proxy, replacing the hand-built codex-args provider.
        assert with_inputs["openai-api-key"] == "${{ steps.codex_oidc.outputs.relay_token }}", job_name
        assert with_inputs["responses-api-endpoint"] == "https://relay-ai.dongwontuna.net/v1/responses", job_name
        assert "codex-args" not in with_inputs, job_name
        # Writable Codex home so the responses-api proxy can write its server-info
        # on the rootless self-hosted runner (default ~/.codex is not writable).
        assert with_inputs["codex-home"] == "${{ runner.temp }}/codex-home", job_name
        assert "env" not in step or "AI_RELAY_API_KEY" not in (step.get("env") or {}), job_name
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


def _assert_pr_head_worktree(job, job_name, head_job):
    checkout_steps = [
        step
        for step in job.get("steps", [])
        if step.get("uses", "").startswith("actions/checkout@")
        and (step.get("with") or {}).get("path") == "pr-head"
    ]
    assert checkout_steps, job_name
    head_checkout = checkout_steps[0]["with"]
    assert head_checkout["repository"] == "${{ needs." + head_job + ".outputs.head_repo_full_name || github.repository }}"
    assert head_checkout["ref"] == "${{ needs." + head_job + ".outputs.head_sha || github.sha }}"
    assert head_checkout["persist-credentials"] is False
    for step in job.get("steps", []):
        if step.get("uses") == CODEX_ACTION:
            with_inputs = step["with"]
            assert with_inputs["working-directory"] == "${{ github.workspace }}/pr-head", job_name
            assert with_inputs["prompt-file"].startswith("${{ github.workspace }}/"), job_name
            assert with_inputs["output-file"].startswith("${{ github.workspace }}/"), job_name
            assert with_inputs["output-schema-file"].startswith("${{ github.workspace }}/"), job_name


def test_stage01_to_stage04_model_jobs_use_pr_head_worktree():
    # Review-stage model jobs source the PR head from guard_and_event;
    # design-stage jobs source it from guard_and_inputs. Both split workflows.
    review_jobs = load_review()["jobs"]
    for job_name in ["review_axes", "techlead"]:
        _assert_pr_head_worktree(review_jobs[job_name], job_name, "guard")
    design_jobs = load_design()["jobs"]
    for job_name in ["design_context", "prepare_clusters", "analyze_clusters", "draft_plan", "chief_decision"]:
        _assert_pr_head_worktree(design_jobs[job_name], job_name, "guard")


def test_stage01_to_stage04_validators_receive_pr_head_repo_path():
    text = pipeline_text()
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


def test_model_jobs_mint_relay_token_locally_via_oidc():
    # Each job that runs codex-action mints its relay key once with the local
    # OIDC helper (no external setup-codex-relay action remains).
    text = pipeline_text()
    assert "setup-codex-relay" not in text
    jobs = pipeline_jobs()
    for job_name, job in jobs.items():
        action_steps = [step for step in job.get("steps", []) if step.get("uses") == CODEX_ACTION]
        if not action_steps:
            continue
        mint_steps = [
            step
            for step in job.get("steps", [])
            if step.get("id") == "codex_oidc"
            and "oidc relay-token" in str(step.get("run", ""))
        ]
        assert len(mint_steps) == 1, job_name


def test_workflow_generates_openai_strict_schemas_for_codex_action():
    text = pipeline_text()
    steps = pipeline_codex_action_steps()
    assert text.count("schema openai-strict --schema") >= len(steps)
    for _, step in steps:
        schema_file = step["with"]["output-schema-file"]
        schema_name = Path(schema_file).name.removesuffix(".openai.schema.json")
        assert f"schema openai-strict --schema {schema_name}" in text


def test_workflow_has_no_model_runner_default_or_codex_cli_env_contract():
    text = pipeline_text()
    assert "CODEX_REVIEW_MODEL_COMMAND" not in text
    assert "CODEX_REVIEW_CODEX_ARGS_JSON" not in text
    assert "codex-review-model-runner" not in text


def test_fix_and_stage07_use_pr_head_worktree():
    text = FIX.read_text(encoding="utf-8")
    assert "path: pr-head" in text
    assert "stage05 prepare-agents" in text and "--repo-path pr-head" in text
    assert "stage06 premerge" in text and "--repo-path pr-head" in text
    assert "stage06 build-semantic-safety-prompt" in text and "--repo-path pr-head" in text
    assert "stage07 validate-fix" in text and "--repo-path pr-head" in text


def test_no_token_validation_job_does_not_request_app_token():
    text = FIX.read_text(encoding="utf-8")
    section = text.split("validate_patch:", 1)[1].split("commit_push:", 1)[0]
    assert "auth app-token" not in section
    assert "GITHUB_TOKEN:" not in section
    assert "stage07 validate-fix" in section
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in section
    assert "stage02 write-deferred-outputs" not in section


def test_semantic_patch_safety_model_gates_stage07_push_validation():
    text = FIX.read_text(encoding="utf-8")
    jobs = load_fix()["jobs"]
    assert "patch_safety" in jobs
    semantic = jobs["patch_safety"]
    assert semantic["needs"] == ["guard", "merge_fixes"]
    semantic_text = text.split("patch_safety:", 1)[1].split("validate_patch:", 1)[0]
    assert "stage06 build-semantic-safety-prompt" in semantic_text
    assert "schema openai-strict --schema stage06-semantic-patch-safety.v1" in semantic_text
    assert "stage06 validate-semantic-safety" in semantic_text
    assert "stage06 write-semantic-safety-outputs" in semantic_text
    assert "oidc relay-token" in semantic_text
    validate_job = jobs["validate_patch"]
    assert validate_job["needs"] == ["guard", "merge_fixes", "patch_safety"]


def test_push_and_issue_fallback_are_default_actual_write_paths():
    push_flag = "CODEX_REVIEW" + "_ENABLE" + "_PUSH"
    issue_flag = "CODEX_REVIEW" + "_ENABLE" + "_ISSUE_FALLBACK"
    assert push_flag not in pipeline_text()
    assert issue_flag not in pipeline_text()

    # stage02 deferred-output emission now lives in the split review workflow.
    assert "stage02 write-deferred-outputs" in pipeline_text()

    # Validation + push now live in the fix workflow.
    fix_text = FIX.read_text(encoding="utf-8")
    validate_section = fix_text.split("validate_patch:", 1)[1].split("commit_push:", 1)[0]
    assert "stage07 validate-fix --dry-run" not in validate_section
    assert "--semantic-safety trusted/codex-review-artifacts/stage06/semantic-safety.json" in validate_section
    assert "stage07 write-validation-outputs" in validate_section
    assert "requires_push_token" in validate_section

    push_section = fix_text.split("commit_push:", 1)[1]
    assert "if: always() && needs.validate_patch.outputs.requires_push_token == 'true'" in push_section
    assert "auth app-token --mode push" in push_section
    assert "stage07 commit-push --in" in push_section
    assert "stage07 push --dry-run" not in push_section
    assert "record_reentry:" not in fix_text

    # Issue fallback (default actual write) now lives in the dedicated issue workflow.
    issue_section = ISSUE.read_text(encoding="utf-8").split("publish_issue:", 1)[1]
    assert "auth app-token --mode stage09" in issue_section
    assert "stage09 apply --in" in issue_section
    assert "stage09 apply --dry-run" not in issue_section


def test_workflow_installs_helper_dependencies_and_pins_python_runtime():
    text = pipeline_text()
    assert "actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405" in text
    assert "python-version: '3.11'" in text
    assert "pip install --disable-pip-version-check -e workflow-helper/setup/codex-review" in text
    assert "pip install --disable-pip-version-check -e trusted/setup/codex-review" not in text


def test_workflow_helper_checkout_uses_workflow_sha_without_changing_base_ref():
    text = pipeline_text()
    assert "ref: ${{ github.event.pull_request.base.sha || github.sha }}" in text
    assert "repository: ${{ github.repository }}" in text
    assert "ref: ${{ github.workflow_sha }}" in text
    assert "path: workflow-helper" in text

    jobs = pipeline_jobs()
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
        for job_name, step in iter_all_steps()
        if step.get("uses") == "actions/setup-python@a309ff8b426b58ec0e2a45f0f869d46889d02405"
    ]
    assert setup_steps
    for job_name, step in setup_steps:
        assert step["with"]["python-version"] == "3.11", job_name
        assert step["with"]["cache"] == "pip", job_name
        assert step["with"]["cache-dependency-path"] == "workflow-helper/setup/codex-review/pyproject.toml", job_name


def test_workflow_never_executes_helper_from_pr_head_or_stale_trusted_tree():
    text = pipeline_text()
    assert "pr-head/setup/codex-review" not in text
    assert "trusted/setup/codex-review" not in text


def test_autofix_path_is_same_repo_and_pr_head_checkout_is_explicit():
    text = FIX.read_text(encoding="utf-8")
    # Fork PRs are blocked in the guard job (head repo must equal the base repo).
    guard_section = text.split("  guard:", 1)[1].split("plan_tasks:", 1)[0]
    assert '"$HEAD_REPO" != "$REPO"' in guard_section
    assert "repository: ${{ needs.guard.outputs.head_repo_full_name || github.repository }}" in text
    assert "ref: ${{ needs.guard.outputs.head_sha || github.sha }}" in text



def test_stage09_issue_fallback_uses_app_token_and_never_github_token_write():
    text = ISSUE.read_text(encoding="utf-8")
    # Reason inference + plan + codex content live in the read-only model job;
    # the actual issue write happens in issue_publish via the app token.
    assert "stage09 infer-reason" in text
    assert "stage09 plan" in text
    assert "stage09 compose" in text
    assert "stage09 apply --in" in text
    assert "auth app-token --mode stage09" in text
    issue_flag = "CODEX_REVIEW" + "_ENABLE" + "_ISSUE_FALLBACK"
    assert issue_flag not in text
    assert "stage09 apply --dry-run" not in text
    # GITHUB_TOKEN job permission is never issues:write; the app token does the write.
    assert "issues: write" not in text


def test_fix_model_commands_run_from_trusted_checkout_not_pr_head():
    text = FIX.read_text(encoding="utf-8")
    section = text.split("run_agents:", 1)[1].split("merge_fixes:", 1)[0]
    assert "working-directory: ${{ github.workspace }}/pr-head" in section
    assert "CODEX_REVIEW_MODEL_CWD" not in section
    assert "CODEX_REVIEW_TARGET_REPO_PATH" not in section


def test_guards_allow_trusted_pipeline_bot_through_permission_check():
    # The loop chains stages via labels attached by the trusted app bot, which is
    # NOT a write collaborator. Every guard that enforces the collaborator
    # write-permission check must also bypass it for that bot, or the automated
    # review->design->fix loop dies after the first human-attached label.
    for path in (REVIEW, DESIGN, FIX, ISSUE):
        text = path.read_text(encoding="utf-8")
        if "collaborators/$ACTOR/permission" not in text:
            continue
        assert '"$ACTOR" = "codex-reviewer-for-dongwonttuna[bot]"' in text or \
               '"$ACTOR" != "codex-reviewer-for-dongwonttuna[bot]"' in text, path.name


def test_review_finalize_routes_upstream_breakage_to_issue_not_lgtm():
    # apply_threads has no route gating, so a non-success result means stage00
    # broke; finalize must route to needs-issue rather than defaulting to lgtm.
    text = REVIEW.read_text(encoding="utf-8")
    assert '[ "$RESOLVE_RESULT" != "success" ]' in text


def test_small_intra_workflow_handoffs_use_job_outputs_not_artifacts():
    # Hybrid data flow: small intra-workflow handoffs move to job outputs via
    # `io to-output`; the corresponding intra artifacts are removed. Cross-workflow,
    # matrix fan-in, and large payload artifacts intentionally remain.
    text = pipeline_text()
    for removed in [
        "codex-review-stage00-collect",
        "codex-review-stage00-model",
        "codex-review-stage03-context",
        "codex-review-stage04-model",
        "codex-review-stage06-semantic-safety",
        "codex-issue-plan",
    ]:
        assert removed not in text, removed
    # One export per converted handoff (8 output keys across the 4 workflows).
    assert text.count("io to-output --name ") == 8
    # Large / matrix / cross-workflow artifacts are still passed as artifacts.
    for kept in [
        "codex-review-stage01-combined",  # large combined findings
        "codex-review-stage05-06",        # multi-MB merged patch
        "codex-review-event",             # cross-workflow boundary
    ]:
        assert kept in text, kept
