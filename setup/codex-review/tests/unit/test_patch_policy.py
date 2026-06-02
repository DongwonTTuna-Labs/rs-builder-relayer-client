import pytest
from codex_review.security.patch_policy import parse_patch_touched_files, validate_patch_policy

PATCH="""diff --git a/src/a.py b/src/a.py
--- a/src/a.py
+++ b/src/a.py
@@ -1 +1 @@
-old
+new
"""
POLICY={"allowed_prefixes":["src/"],"forbidden_prefixes":[".github/workflows/"],"max_patch_bytes":10000,"max_files":3,"dangerous_keywords":["secret","auth","nonce"]}


def test_patch_policy_accepts_safe_patch():
    report=validate_patch_policy(PATCH, POLICY, {})
    assert report["touched_files"] == ["src/a.py"]


def test_patch_policy_rejects_forbidden_path():
    bad=PATCH.replace("src/a.py", ".github/workflows/x.yml")
    with pytest.raises(Exception):
        validate_patch_policy(bad, POLICY, {})


def test_patch_policy_rejects_secret_like_addition():
    bad=PATCH.replace("+new", "+api_key='abcdefghijklmnopqrstuvwx'")
    with pytest.raises(Exception):
        validate_patch_policy(bad, POLICY, {})


def test_semantic_keywords_are_advisory_not_hard_blockers():
    patch = PATCH.replace("+new", "+OpenSpec is authoritative and nonce handling is covered by tests")
    report = validate_patch_policy(patch, POLICY, {})
    assert report["ok"] is True
    assert report["semantic_risk_warning_count"] >= 2
    kinds = {item["kind"] for item in report["semantic_risk_warnings"]}
    assert "semantic_keyword" in kinds
    assert "public_api_or_protocol_semantic_risk" in kinds


def test_parse_patch_touched_files_normalizes_paths():
    assert parse_patch_touched_files(PATCH) == ["src/a.py"]
