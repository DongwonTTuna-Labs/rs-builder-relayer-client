from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOW = ROOT / ".github" / "workflows" / "codex-review-orchestrator.yml"
CODEX_ACTION = "openai/codex-action@e0fdf01220eb9a88167c4898839d273e3f2609d1"


def jobs():
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]


def test_model_jobs_have_no_repo_write_permissions():
    for name, job in jobs().items():
        uses_model = any(step.get("uses") == CODEX_ACTION for step in job.get("steps", []))
        if "model" not in name and not uses_model:
            continue
        perms = job.get("permissions", {})
        assert perms.get("contents") == "read"
        assert perms.get("pull-requests") == "read"
        assert perms.get("id-token") == "write"
        assert perms.get("issues") != "write"


def test_trusted_write_jobs_keep_github_token_read_only():
    write_jobs = ["resolve_apply_trusted", "review_publish_trusted", "design_publish_trusted", "issue_fallback_trusted", "push_trusted"]
    for name in write_jobs:
        perms = jobs()[name].get("permissions", {})
        assert perms.get("contents") == "read"
        assert perms.get("pull-requests") == "read"
        assert perms.get("issues") == "read"
        assert "write" not in set(perms.values())


def test_no_token_validation_job_is_read_only():
    perms = jobs()["push_validate_no_token"].get("permissions", {})
    assert perms.get("contents") == "read"
    assert perms.get("pull-requests") == "read"
    assert perms.get("issues") == "read"
    assert "write" not in set(perms.values())


def test_write_jobs_use_app_token_not_github_token_write_permissions():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "auth app-token --mode stage00" in text
    assert "auth app-token --mode stage02" in text
    assert "auth app-token --mode stage04" in text
    assert "auth app-token --mode push" in text
    assert "auth app-token --mode stage08" not in text
    assert "auth app-token --mode stage09" in text
    assert "GITHUB_TOKEN: ${{ github.token }}" not in text.split("resolve_apply_trusted:", 1)[1]
    for name in ["resolve_apply_trusted", "review_publish_trusted", "design_publish_trusted", "issue_fallback_trusted", "push_trusted"]:
        section = text.split(f"  {name}:", 1)[1].split("\n  ", 1)[0]
        assert "write" not in section


def test_app_token_permission_metadata_is_threaded_to_write_commands():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "write_output(\"permissions_json\"" not in text  # helper code stays outside workflow
    assert text.count("CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON: ${{ steps.app_token.outputs.permissions_json }}") >= 4


def test_app_token_steps_include_current_codex_app_secret_fallbacks():
    app_token_steps = []
    for job in jobs().values():
        for step in job.get("steps", []):
            if "codex-review auth app-token" in str(step.get("run", "")):
                app_token_steps.append(step)

    assert len(app_token_steps) == 5
    for step in app_token_steps:
        env = step.get("env", {})
        assert env.get("CODEX_APP_ID") == "${{ secrets.CODEX_APP_ID }}"
        assert env.get("CODEX_APP_PRIVATE_KEY") == "${{ secrets.CODEX_APP_PRIVATE_KEY }}"


def test_model_jobs_use_oidc_relay_without_write_permissions():
    model_jobs = [
        "resolve_triage_model",
        "review_axes_model",
        "techlead_model",
        "design_model_chain",
        "design_chief_model",
        "fix_agent_model",
        "fix_collect_and_merge",
    ]
    for name in model_jobs:
        job = jobs()[name]
        steps = job.get("steps", [])
        relay_steps = [
            step for step in steps
            if step.get("uses") == "DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main"
        ]
        assert len(relay_steps) == 1
        assert relay_steps[0]["with"]["trusted-actors"] == "DongwonTTuna,codex-reviewer-for-dongwonttuna[bot]"
        relay_home = relay_steps[0]["with"]["codex-home"]
        assert "codex-relay-home-" in relay_home

        action_steps = [step for step in steps if step.get("uses") == CODEX_ACTION]
        assert action_steps
        for step in action_steps:
            assert "openai-api-key" not in step["with"]
            assert "responses-api-endpoint" not in step["with"]
            assert step["with"]["codex-args"] == "${{ steps.relay-token.outputs.codex_args }}"
            assert step["env"]["AI_RELAY_API_KEY"] == "${{ steps.relay-token.outputs.relay_token }}"
            assert step["with"]["output-schema-file"]
            assert step["with"]["codex-home"] == relay_home
