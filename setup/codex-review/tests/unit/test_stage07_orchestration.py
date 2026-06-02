from codex_review.stages.stage07_push.orchestrate import commit_and_push_validated_fix, run_push_flow


def test_push_flow_no_fix_returns_safe_result(tmp_path):
    result = run_push_flow({"schema_version":"stage06-merged-fix.v1", "status":"no_fix", "patch":""}, {"head_sha":"abc"}, {}, tmp_path, None, dry_run=True)
    assert result["pushed"] is False
    assert result["status"] == "no_fix"


def test_commit_push_preserves_nonvalidated_status_for_issue_fallback(tmp_path):
    result = commit_and_push_validated_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready_to_push", "patch":"diff --git a/src/a.py b/src/a.py\n"},
        {"schema_version":"stage07-validated-fix.v1", "status":"no_diff_repeat", "validated":False},
        {"owner":"o", "repo":"r", "pr_number":1, "head_sha":"abc"},
        {},
        tmp_path,
        token=None,
    )
    assert result["pushed"] is False
    assert result["status"] == "no_diff_repeat"
    assert result["validation_result"]["status"] == "no_diff_repeat"


def test_validate_and_test_reports_already_applied_patch_as_no_diff_repeat(tmp_path):
    import subprocess
    from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix

    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "src").mkdir()
    (repo / "src/a.txt").write_text("old\n", encoding="utf-8")
    subprocess.run(["git", "add", "src/a.txt"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "old"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    patch = """diff --git a/src/a.txt b/src/a.txt
--- a/src/a.txt
+++ b/src/a.txt
@@ -1 +1 @@
-old
+new
"""
    subprocess.run(["git", "apply", "-"], cwd=repo, input=patch, text=True, check=True)
    subprocess.run(["git", "add", "src/a.txt"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "new"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()

    result = validate_and_test_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready", "patch":patch, "expected_head_sha":head},
        {"head_sha":head},
        {"autofix":{"allowed_prefixes":["src/"], "max_patch_bytes":10000}},
        repo,
        dry_run=False,
    )

    assert result["status"] == "no_diff_repeat"
    assert result["validated"] is False


def test_stage07_write_validation_outputs_declares_push_token_requirement(tmp_path, monkeypatch):
    import json
    from codex_review.cli import main

    artifact = tmp_path / "validated-fix.json"
    artifact.write_text(json.dumps({"schema_version":"stage07-validated-fix.v1", "status":"validated", "validated":True}), encoding="utf-8")
    github_output = tmp_path / "github-output.txt"
    monkeypatch.setenv("GITHUB_OUTPUT", str(github_output))

    assert main(["stage07", "write-validation-outputs", "--in", str(artifact)]) == 0
    text = github_output.read_text(encoding="utf-8")
    assert "validation_status=validated" in text
    assert "requires_push_token=true" in text


def test_stage02_write_deferred_outputs_counts_items(tmp_path, monkeypatch):
    import json
    from codex_review.cli import main

    artifact = tmp_path / "review-publication.json"
    artifact.write_text(json.dumps({"schema_version":"stage02-review-publication.v1", "deferred_items":[{"id":"a"}, {"id":"b"}]}), encoding="utf-8")
    github_output = tmp_path / "github-output-stage02.txt"
    monkeypatch.setenv("GITHUB_OUTPUT", str(github_output))

    assert main(["stage02", "write-deferred-outputs", "--in", str(artifact)]) == 0
    text = github_output.read_text(encoding="utf-8")
    assert "has_deferred_issue_items=true" in text
    assert "deferred_issue_count=2" in text


def test_validate_and_test_returns_validation_failed_artifact_instead_of_raising(tmp_path):
    import subprocess
    from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix

    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "src").mkdir()
    (repo / "src/a.txt").write_text("old\n", encoding="utf-8")
    subprocess.run(["git", "add", "src/a.txt"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "old"], cwd=repo, check=True, stdout=subprocess.DEVNULL)

    patch = """diff --git a/src/a.txt b/src/a.txt
--- a/src/a.txt
+++ b/src/a.txt
@@ -1 +1 @@
-old
+new
"""
    result = validate_and_test_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready", "patch":patch, "expected_head_sha":"not-the-current-head"},
        {"head_sha":"not-the-current-head"},
        {"autofix":{"allowed_prefixes":["src/"], "max_patch_bytes":10000}},
        repo,
        dry_run=False,
    )

    assert result["status"] == "validation_failed"
    assert result["validated"] is False
    assert result["pushed"] is False
    assert result["error_type"] == "ValidationError"


def test_ready_status_with_empty_patch_routes_to_empty_patch():
    from codex_review.stages.stage07_push.orchestrate import run_push_flow

    result = run_push_flow(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready_to_push", "patch":""},
        {"head_sha":"abc"},
        {},
        ".",
        None,
        dry_run=True,
    )

    assert result["status"] == "empty_patch"
    assert result["pushed"] is False


def _semantic_approval_for_patch(patch: str) -> dict:
    import hashlib

    return {
        "schema_version": "stage06-semantic-patch-safety.v1",
        "status": "approved",
        "approved": True,
        "patch_hash": hashlib.sha256(patch.encode("utf-8")).hexdigest(),
        "summary": "Exact patch hash reviewed and approved.",
        "blocking_reason": None,
        "reviewed_criteria": ["OpenSpec scope", "no credential exfiltration"],
        "semantic_findings": [],
    }


def test_validate_and_test_requires_exact_semantic_patch_safety_approval(tmp_path):
    import subprocess
    from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix

    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "src").mkdir()
    (repo / "src/a.txt").write_text("old\n", encoding="utf-8")
    subprocess.run(["git", "add", "src/a.txt"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "old"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    patch = """diff --git a/src/a.txt b/src/a.txt
--- a/src/a.txt
+++ b/src/a.txt
@@ -1 +1 @@
-old
+new
"""

    result = validate_and_test_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready", "patch":patch, "expected_head_sha":head},
        {"head_sha":head},
        {"autofix":{"allowed_prefixes":["src/"], "max_patch_bytes":10000}},
        repo,
        dry_run=False,
        semantic_safety={},
    )

    assert result["status"] == "semantic_safety_missing"
    assert result["validated"] is False
    assert result["semantic_safety_approved"] is False


def test_validate_and_test_allows_exact_semantic_patch_safety_approval(tmp_path):
    import subprocess
    from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix

    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "src").mkdir()
    (repo / "src/a.txt").write_text("old\n", encoding="utf-8")
    subprocess.run(["git", "add", "src/a.txt"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "old"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    patch = """diff --git a/src/a.txt b/src/a.txt
--- a/src/a.txt
+++ b/src/a.txt
@@ -1 +1 @@
-old
+new
"""

    result = validate_and_test_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready", "patch":patch, "expected_head_sha":head},
        {"head_sha":head},
        {"autofix":{"allowed_prefixes":["src/"], "max_patch_bytes":10000}},
        repo,
        dry_run=False,
        semantic_safety=_semantic_approval_for_patch(patch),
    )

    assert result["status"] == "validated"
    assert result["validated"] is True
    assert result["semantic_safety_approved"] is True


def test_validate_and_test_accepts_new_file_patch(tmp_path):
    import subprocess
    from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix

    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    subprocess.run(["git", "add", "README.md"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    patch = """diff --git a/docs/CODEX_REVIEW_LGTM_LOOP.md b/docs/CODEX_REVIEW_LGTM_LOOP.md
new file mode 100644
index 0000000..04331b2
--- /dev/null
+++ b/docs/CODEX_REVIEW_LGTM_LOOP.md
@@ -0,0 +1 @@
+LGTM loop smoke
"""

    result = validate_and_test_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready", "patch":patch, "expected_head_sha":head},
        {"head_sha":head},
        {"autofix":{"allowed_prefixes":["docs/"], "max_patch_bytes":10000}},
        repo,
        dry_run=False,
        semantic_safety=_semantic_approval_for_patch(patch),
    )

    assert result["status"] == "validated"
    assert result["validated"] is True
    assert result["semantic_safety_approved"] is True
    assert result["applied_diff_hash"]


def test_commit_push_defensively_refuses_validated_artifact_without_semantic_approval(tmp_path):
    result = commit_and_push_validated_fix(
        {"schema_version":"stage06-merged-fix.v1", "status":"ready_to_push", "patch":"diff --git a/src/a.py b/src/a.py\n"},
        {"schema_version":"stage07-validated-fix.v1", "status":"validated", "validated":True, "semantic_safety_approved":False},
        {"owner":"o", "repo":"r", "pr_number":1, "head_sha":"abc"},
        {},
        tmp_path,
        token=None,
    )
    assert result["status"] == "semantic_safety_missing"
    assert result["pushed"] is False
