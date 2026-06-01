"""Trusted stage07 push orchestration.

Stage07 is deliberately split into two phases:

* validate/test: runs without any write token in the PR-head worktree, applies the
  patch and executes only trusted allowlisted tests with a sanitized env.
* commit/push: runs with the GitHub App installation token, re-checks out the
  exact PR head, reapplies the already validated patch, commits with hooks
  disabled, and pushes. It never runs tests.
"""
from __future__ import annotations

import hashlib
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from codex_review.errors import ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.security.patch_policy import validate_patch_policy
from .apply_patch import apply_merged_patch, collect_applied_diff, run_diff_check
from .commit import build_commit_message, create_commit, validate_commit_diff
from .push import push_commit, verify_pushed_head
from .run_tests import run_required_tests, select_test_commands
from .validate import validate_autofix_commit_cap, validate_current_head, validate_push_target, validate_ready_to_push, validate_worktree_clean
from .safe_subprocess import sanitized_env


def _policy(config: dict[str, Any], merged_fix: dict[str, Any]) -> dict[str, Any]:
    policy = dict(config.get("autofix", {}) or {})
    if isinstance(merged_fix.get("fix_policy"), dict):
        policy.update(merged_fix["fix_policy"])
    if isinstance(merged_fix.get("policy"), dict):
        policy.update(merged_fix["policy"])
    return policy


def _patch_text(merged_fix: dict[str, Any]) -> str:
    if merged_fix.get("patch_path"):
        return Path(merged_fix["patch_path"]).read_text(encoding="utf-8")
    return merged_fix.get("patch") or merged_fix.get("patch_text") or ""


def _sha256_text(text: str) -> str:
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()


def _write_temp_patch(patch_text: str) -> Path:
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".patch", delete=False)
    with handle:
        handle.write(patch_text)
    return Path(handle.name)


def _git(repo_path: str | Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(["git", *args], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    if check and proc.returncode != 0:
        raise ValidationError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc


def _configure_git_author(repo_path: str | Path, policy: dict[str, Any]) -> None:
    name = policy.get("git_author_name", "Codex Review Bot")
    email = policy.get("git_author_email", "codex-review@example.invalid")
    _git(repo_path, "config", "user.name", str(name))
    _git(repo_path, "config", "user.email", str(email))


def _current_head(repo_path: str | Path) -> str:
    return _git(repo_path, "rev-parse", "HEAD").stdout.strip()


def _validate_local_expected_head(repo_path: str | Path, pr_context: dict[str, Any], merged_fix: dict[str, Any]) -> str:
    expected = str(merged_fix.get("expected_head_sha") or pr_context.get("head_sha") or "")
    current = _current_head(repo_path)
    if expected and current != expected:
        raise ValidationError(f"local PR-head checkout does not match expected head: expected {expected}, current {current}")
    return current


def _ready_or_noop(merged_fix: dict[str, Any], patch: str) -> dict[str, Any] | None:
    status = merged_fix.get("status")
    if status in {"no_fix", "blocked"} or not patch:
        return {
            "schema_version": "stage07-push-result.v1",
            "status": status or "no_fix",
            "pushed": False,
            "commit_sha": None,
            "reason": "no merged patch is ready to push",
        }
    return None


def validate_and_test_fix(
    merged_fix: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    """Apply and test a patch without any write token.

    The caller must run this in a PR-head checkout with no persisted credentials.
    """
    patch = _patch_text(merged_fix)
    noop = _ready_or_noop(merged_fix, patch)
    if noop:
        return {**noop, "schema_version": "stage07-validated-fix.v1", "validated": False}

    validate_ready_to_push({**merged_fix, "patch": patch})
    head = _validate_local_expected_head(repo_path, pr_context, merged_fix)
    policy = _policy(config, merged_fix)
    policy_report = validate_patch_policy(patch, policy, {"repo_path": repo_path})
    tests = select_test_commands(merged_fix, config)

    if dry_run:
        return {
            "schema_version": "stage07-validated-fix.v1",
            "status": "dry_run",
            "validated": False,
            "pushed": False,
            "commit_sha": None,
            "head_sha": head,
            "patch_hash": _sha256_text(patch),
            "policy_report": policy_report,
            "tests": tests,
        }

    validate_worktree_clean(repo_path)
    patch_path = _write_temp_patch(patch)
    try:
        apply_report = apply_merged_patch(patch_path, repo_path)
        run_diff_check(repo_path)
        applied_diff = collect_applied_diff(repo_path)
        applied_policy_report = validate_patch_policy(applied_diff, policy, {})
        test_report = run_required_tests(tests, repo_path)
        status = "validated" if test_report.get("passed", True) else "tests_failed"
        return {
            "schema_version": "stage07-validated-fix.v1",
            "status": status,
            "validated": status == "validated",
            "pushed": False,
            "commit_sha": None,
            "head_sha": head,
            "patch_hash": _sha256_text(patch),
            "applied_diff_hash": _sha256_text(applied_diff),
            "apply_report": apply_report,
            "policy_report": applied_policy_report,
            "test_report": test_report,
        }
    finally:
        try:
            patch_path.unlink(missing_ok=True)
        except Exception:
            pass


def commit_validated_fix(merged_fix: dict[str, Any], pr_context: dict[str, Any], config: dict[str, Any], repo_path: str | Path) -> dict[str, Any]:
    """Commit an already-applied and validated fix in the trusted checkout."""
    policy = _policy(config, merged_fix)
    old_head = pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or _current_head(repo_path)
    _configure_git_author(repo_path, policy)
    message = build_commit_message(merged_fix, str(merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash") or "unknown"), str(old_head))
    commit_sha = create_commit(repo_path, message)
    report = validate_commit_diff(repo_path, commit_sha, policy)
    return {"schema_version": "stage07-push-result.v1", "status": "committed", "pushed": False, "commit_sha": commit_sha, "policy_report": report}


def commit_and_push_validated_fix(
    merged_fix: dict[str, Any],
    validation_result: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    token: str | None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    """Reapply a validated patch, commit it, and push with an App token.

    This phase intentionally does not execute tests. It verifies that the diff
    produced by applying the patch matches the no-token validation phase.
    """
    patch = _patch_text(merged_fix)
    noop = _ready_or_noop(merged_fix, patch)
    if noop:
        return noop

    if not validation_result.get("validated"):
        raise ValidationError("stage07 commit/push requires a successful no-token validation artifact")
    if validation_result.get("patch_hash") and validation_result["patch_hash"] != _sha256_text(patch):
        raise ValidationError("merged patch changed after no-token validation")

    validate_push_target(pr_context)
    owner = str(pr_context.get("owner"))
    repo = str(pr_context.get("repo"))
    pr_number = int(pr_context.get("pr_number"))
    if not token:
        raise ValidationError("stage07 actual push requires a GitHub App installation token")
    assert_installation_token_for_repo(token, owner, repo, permissions_for_write_mode("push"))
    validate_current_head(pr_context, merged_fix, token)
    _validate_local_expected_head(repo_path, pr_context, merged_fix)

    policy = _policy(config, merged_fix)
    cap_report = validate_autofix_commit_cap(owner, repo, pr_number, token, policy)
    validate_worktree_clean(repo_path)
    validate_patch_policy(patch, policy, {"repo_path": repo_path})

    if dry_run:
        return {
            "schema_version": "stage07-push-result.v1",
            "status": "dry_run_commit_push",
            "pushed": False,
            "commit_sha": None,
            "patch_hash": _sha256_text(patch),
            "commit_cap": cap_report,
        }

    patch_path = _write_temp_patch(patch)
    old_head = pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or _current_head(repo_path)
    try:
        apply_report = apply_merged_patch(patch_path, repo_path)
        run_diff_check(repo_path)
        applied_diff = collect_applied_diff(repo_path)
        applied_hash = _sha256_text(applied_diff)
        expected_applied_hash = validation_result.get("applied_diff_hash")
        if expected_applied_hash and expected_applied_hash != applied_hash:
            raise ValidationError("applied diff differs from no-token validation artifact")
        applied_policy_report = validate_patch_policy(applied_diff, policy, {})
        _configure_git_author(repo_path, policy)
        message = build_commit_message(merged_fix, str(merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash") or "unknown"), str(old_head))
        commit_sha = create_commit(repo_path, message)
        commit_policy_report = validate_commit_diff(repo_path, commit_sha, policy)
        head_ref = pr_context.get("head_ref")
        if not head_ref:
            return {"schema_version": "stage07-push-result.v1", "status": "committed_no_head_ref", "pushed": False, "commit_sha": commit_sha, "policy_report": commit_policy_report}
        push_report = push_commit(repo_path, str(head_ref), owner, repo, token)
        pushed = bool(push_report.get("pushed"))
        verified = False
        if pushed:
            verified = verify_pushed_head(owner, repo, pr_number, commit_sha, token)
            if not verified:
                raise ValidationError("pushed commit could not be verified as current PR head")
        return {
            "schema_version": "stage07-push-result.v1",
            "status": "pushed" if pushed else "push_failed",
            "pushed": pushed,
            "commit_sha": commit_sha,
            "verified": verified,
            "apply_report": apply_report,
            "policy_report": commit_policy_report,
            "applied_policy_report": applied_policy_report,
            "push_report": push_report,
            "commit_cap": cap_report,
            "validation_result": {"status": validation_result.get("status"), "applied_diff_hash": validation_result.get("applied_diff_hash")},
        }
    finally:
        try:
            patch_path.unlink(missing_ok=True)
        except Exception:
            pass


def run_push_flow(
    merged_fix: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    token: str | None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    """Backward-compatible wrapper for older callers.

    In dry-run mode this only validates. In actual mode it still sanitizes the
    test environment, but production workflows should prefer the explicit
    validate/test + commit/push split.
    """
    if dry_run:
        validation = validate_and_test_fix(merged_fix, pr_context, config, repo_path, dry_run=True)
        status = validation.get("status") if validation.get("status") in {"no_fix", "blocked"} else "dry_run"
        return {**validation, "schema_version": "stage07-push-result.v1", "status": status, "pushed": False}
    validation = validate_and_test_fix(merged_fix, pr_context, config, repo_path, dry_run=False)
    return commit_and_push_validated_fix(merged_fix, validation, pr_context, config, repo_path, token, dry_run=False)
