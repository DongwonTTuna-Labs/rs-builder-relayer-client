import json

from codex_review.cli import main


def write_json(path, payload):
    path.write_text(json.dumps(payload), encoding="utf-8")


def test_stage03_prompt_only_commands_write_model_prompts(tmp_path):
    context = tmp_path / "design-context.json"
    inventory = tmp_path / "design-inventory.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(
        context,
        {
            "schema_version": "stage03-design-context.v1",
            "findings": [{"finding_id": "f1", "summary": "needs design"}],
            "techlead_decision": {"decisions": [{"finding_id": "f1", "action": "needs_design"}]},
        },
    )
    write_json(inventory, {"schema_version": "stage03-design-inventory.v1", "items": [{"finding_id": "f1", "summary": "needs design"}]})
    write_json(clusters, {"schema_version": "stage03-design-clusters.v1", "clusters": [{"cluster_id": "c1", "finding_ids": ["f1"]}]})
    write_json(analysis, {"schema_version": "stage03-cluster-analysis.v1", "analyses": [{"cluster_id": "c1", "status": "ready"}]})

    cases = [
        (["stage03", "build-inventory-prompt", "--in", str(context)], "stage03-design-inventory.v1"),
        (["stage03", "build-clusters-prompt", "--inventory", str(inventory), "--pr-context", str(context)], "stage03-design-clusters.v1"),
        (["stage03", "build-analysis-prompt", "--inventory", str(clusters), "--pr-context", str(context)], "stage03-cluster-analysis.v1"),
        (["stage03", "build-plan-prompt", "--pr-context", str(context), "--inventory", str(clusters), "--result", str(analysis)], "stage03-design-plan.v1"),
    ]
    for idx, (args, expected) in enumerate(cases, 1):
        out = tmp_path / f"prompt-{idx}.md"
        assert main([*args, "--out", str(out)]) == 0
        assert expected in out.read_text(encoding="utf-8")


def test_stage03_plan_prompt_keeps_human_routing_in_stage04(tmp_path):
    context = tmp_path / "design-context.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(context, {"schema_version": "stage03-design-context.v1", "findings": [{"finding_id": "f1"}]})
    write_json(clusters, {"schema_version": "stage03-design-clusters.v1", "clusters": [{"cluster_id": "c1"}]})
    write_json(analysis, {"schema_version": "stage03-cluster-analysis.v1", "analyses": [{"cluster_id": "c1"}]})

    out = tmp_path / "prompt.md"
    assert main([
        "stage03",
        "build-plan-prompt",
        "--pr-context",
        str(context),
        "--inventory",
        str(clusters),
        "--result",
        str(analysis),
        "--out",
        str(out),
    ]) == 0

    prompt = out.read_text(encoding="utf-8")
    assert "candidate design plan" in prompt
    assert "stage04" in prompt
    assert "open_questions" not in prompt


def test_openspec_backed_prompts_drive_closed_implementation_plan(tmp_path):
    context = tmp_path / "design-context.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(
        context,
        {
            "schema_version": "stage03-design-context.v1",
            "openspec_backed": True,
            "openspec_context": {"source_summary": ["openspec/changes/demo/tasks.md"]},
            "findings": [{"finding_id": "f1"}],
        },
    )
    write_json(clusters, {"schema_version": "stage03-design-clusters.v1", "clusters": [{"cluster_id": "c1"}]})
    write_json(analysis, {"schema_version": "stage03-cluster-analysis.v1", "analyses": [{"cluster_id": "c1"}]})

    out = tmp_path / "prompt.md"
    assert main([
        "stage03",
        "build-plan-prompt",
        "--pr-context",
        str(context),
        "--inventory",
        str(clusters),
        "--result",
        str(analysis),
        "--out",
        str(out),
    ]) == 0

    prompt = out.read_text(encoding="utf-8")
    assert "OpenSpec-backed implementation" in prompt
    assert "acceptance_criteria" in prompt
    assert "questions" not in prompt.lower()


def test_stage05_prepare_agents_writes_prompts_matrix_and_github_outputs(tmp_path, monkeypatch):
    manifest = tmp_path / "manifest.json"
    design_plan = tmp_path / "design-plan.json"
    chief = tmp_path / "chief.json"
    docs = tmp_path / "docs.md"
    matrix = tmp_path / "matrix.json"
    gh_output = tmp_path / "github-output"
    work_dir = tmp_path / "agents"
    write_json(manifest, {"schema_version": "stage05-fix-task-manifest.v1", "tasks": [{"task_id": "fix/one", "summary": "Fix it", "allowed_files": ["src/lib.rs"]}]})
    write_json(design_plan, {"schema_version": "stage03-design-plan.v1", "edit_sequence": [], "tests": ["cargo test"]})
    write_json(chief, {"schema_version": "stage04-design-chief-decision.v1", "status": "approved_for_fix", "fix_policy": {"allowed_files": ["src/lib.rs"]}})
    docs.write_text("docs", encoding="utf-8")
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "stage05",
        "prepare-agents",
        "--inventory",
        str(manifest),
        "--design-plan",
        str(design_plan),
        "--chief-decision",
        str(chief),
        "--docs-context",
        str(docs),
        "--repo-path",
        "pr-head",
        "--work-dir",
        str(work_dir),
        "--out",
        str(matrix),
    ]) == 0

    payload = json.loads(matrix.read_text(encoding="utf-8"))
    assert payload["include"][0]["task_id"] == "fix/one"
    assert payload["include"][0]["task_path"] == "fix_one"
    prompt_path = work_dir / "fix_one" / "prompt.md"
    task_path = work_dir / "fix_one" / "task.json"
    assert prompt_path.is_file()
    assert task_path.is_file()
    assert "src/lib.rs" in prompt_path.read_text(encoding="utf-8")
    output_text = gh_output.read_text(encoding="utf-8")
    assert "has_agent_tasks=true" in output_text
    assert "agent_matrix=" in output_text


def test_stage06_prepare_merge_model_routes_without_invoking_model_when_clean(tmp_path, monkeypatch):
    premerge = tmp_path / "premerge.json"
    collection = tmp_path / "collection.json"
    pr_context = tmp_path / "pr-context.json"
    raw_out = tmp_path / "merged-fix.raw.json"
    route = tmp_path / "merge-route.json"
    gh_output = tmp_path / "github-output"
    write_json(premerge, {"schema_version": "stage06-premerge-report.v1", "clean": True, "results": []})
    write_json(collection, {"schema_version": "stage05-fix-collection-result.v1", "results": []})
    write_json(pr_context, {"head_sha": "abc123"})
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "stage06",
        "prepare-merge-model",
        "--inventory",
        str(premerge),
        "--in",
        str(collection),
        "--pr-context",
        str(pr_context),
        "--raw-out",
        str(raw_out),
        "--out",
        str(route),
    ]) == 0

    assert json.loads(route.read_text(encoding="utf-8"))["needs_model"] is False
    assert json.loads(raw_out.read_text(encoding="utf-8"))["status"] == "no_fix"
    assert "needs_model=false" in gh_output.read_text(encoding="utf-8")


def test_stage06_prepare_merge_model_writes_prompt_for_conflicts(tmp_path, monkeypatch):
    premerge = tmp_path / "premerge.json"
    collection = tmp_path / "collection.json"
    pr_context = tmp_path / "pr-context.json"
    docs = tmp_path / "docs.md"
    raw_out = tmp_path / "merged-fix.raw.json"
    prompt_out = tmp_path / "merge-prompt.md"
    route = tmp_path / "merge-route.json"
    gh_output = tmp_path / "github-output"
    write_json(premerge, {"schema_version": "stage06-premerge-report.v1", "clean": False, "results": [{"index": 0, "ok": False}]})
    write_json(collection, {"schema_version": "stage05-fix-collection-result.v1", "results": [{"task_id": "t1", "status": "patched", "patch": "diff --git a/x b/x\n"}]})
    write_json(pr_context, {"head_sha": "abc123"})
    docs.write_text("docs", encoding="utf-8")
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "stage06",
        "prepare-merge-model",
        "--inventory",
        str(premerge),
        "--in",
        str(collection),
        "--pr-context",
        str(pr_context),
        "--docs-context",
        str(docs),
        "--prompt-out",
        str(prompt_out),
        "--raw-out",
        str(raw_out),
        "--out",
        str(route),
    ]) == 0

    assert json.loads(route.read_text(encoding="utf-8"))["needs_model"] is True
    assert prompt_out.is_file()
    assert not raw_out.exists()
    assert "stage06-merged-fix.v1" in prompt_out.read_text(encoding="utf-8")
    assert "needs_model=true" in gh_output.read_text(encoding="utf-8")
