from _pipeline import CODEX_ACTION, all_jobs, all_text, iter_all_steps


def jobs():
    return all_jobs()


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
    write_jobs = ["apply_threads", "publish_review", "publish_design", "commit_push"]
    for name in write_jobs:
        perms = jobs()[name].get("permissions", {})
        assert perms.get("contents") == "read"
        assert perms.get("pull-requests") == "read"
        assert perms.get("issues") == "read"
        assert "write" not in set(perms.values())


def test_no_token_validation_job_is_read_only():
    # Merge/safety/validation share one job that runs models (needs id-token: write
    # for the OIDC relay token) but must never hold repo-write permissions.
    perms = jobs()["merge_validate"].get("permissions", {})
    assert perms.get("contents") == "read"
    assert perms.get("pull-requests") == "read"
    assert perms.get("issues") == "read"
    assert perms.get("id-token") == "write"
    repo_write_scopes = {k: v for k, v in perms.items() if k != "id-token"}
    assert "write" not in set(repo_write_scopes.values())


def test_write_jobs_use_app_token_not_github_token_write_permissions():
    text = all_text()
    assert "auth app-token --mode stage00" in text
    assert "auth app-token --mode stage02" in text
    assert "auth app-token --mode stage04" in text
    assert "auth app-token --mode push" in text
    assert "auth app-token --mode stage08" not in text
    assert "auth app-token --mode stage09" in text
    # Trusted write/publish jobs must drive writes with the app token, never GITHUB_TOKEN.
    all_jobs_map = jobs()
    for name in ["apply_threads", "publish_review", "publish_design", "commit_push"]:
        for step in all_jobs_map[name].get("steps", []):
            env = step.get("env") or {}
            assert env.get("GITHUB_TOKEN") != "${{ github.token }}", name


def test_app_token_permission_metadata_is_threaded_to_write_commands():
    text = all_text()
    assert "write_output(\"permissions_json\"" not in text  # helper code stays outside workflow
    assert text.count("CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON: ${{ steps.app_token.outputs.permissions_json }}") >= 4


def test_app_token_steps_include_current_codex_app_secret_fallbacks():
    # iter_all_steps walks each file's jobs directly (job names like finalize_labels
    # collide across files, so a merged-by-name view would undercount).
    app_token_steps = [
        step
        for _, step in iter_all_steps()
        if "codex-review auth app-token" in str(step.get("run", ""))
    ]

    # review: stage00, stage02, label-ops; design: stage04, label-ops;
    # fix: push, loop-state, label-ops; orchestrator: stage09.
    assert len(app_token_steps) == 9
    for step in app_token_steps:
        env = step.get("env", {})
        assert env.get("CODEX_APP_ID") == "${{ secrets.CODEX_APP_ID }}"
        assert env.get("CODEX_APP_PRIVATE_KEY") == "${{ secrets.CODEX_APP_PRIVATE_KEY }}"


def test_model_jobs_use_native_codex_action_relay_without_write_permissions():
    model_jobs = [
        "triage_threads",
        "review_axes",
        "techlead",
        "prepare_clusters",
        "analyze_clusters",
        "draft_plan",
        "chief_decision",
        "run_agents",
        "merge_validate",
    ]
    for name in model_jobs:
        job = jobs()[name]
        steps = job.get("steps", [])
        mint_steps = [
            step for step in steps
            if step.get("id") == "codex_oidc" and "oidc relay-token" in str(step.get("run", ""))
        ]
        assert len(mint_steps) == 1
        # No external relay action remains; the read-only model job mints the key itself.
        assert all(
            step.get("uses") != "DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main"
            for step in steps
        )

        action_steps = [step for step in steps if step.get("uses") == CODEX_ACTION]
        assert action_steps
        for step in action_steps:
            assert step["with"]["openai-api-key"] == "${{ steps.codex_oidc.outputs.relay_token }}"
            assert step["with"]["responses-api-endpoint"] == "https://relay-ai.dongwontuna.net/v1/responses"
            assert "codex-args" not in step["with"]
            assert step["with"]["codex-home"].startswith("${{ runner.temp }}/codex-home")
            assert "AI_RELAY_API_KEY" not in (step.get("env") or {})
            assert step["with"]["output-schema-file"]
