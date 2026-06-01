from __future__ import annotations

from codex_review.config import load_config
from codex_review.stages.stage07_push.validate import validate_autofix_commit_cap


def test_default_autofix_commit_cap_is_unlimited():
    config = load_config()
    assert config["autofix"]["max_commits"] == 0


def test_zero_autofix_commit_cap_skips_remote_commit_listing(monkeypatch):
    called = False

    def fail_if_called(*args, **kwargs):
        nonlocal called
        called = True
        raise AssertionError("commit listing should not run when max_commits is unlimited")

    monkeypatch.setattr("codex_review.stages.stage07_push.validate.list_pull_request_commits", fail_if_called)

    result = validate_autofix_commit_cap("owner", "repo", 1, "token", {"max_commits": 0})

    assert called is False
    assert result["checked"] is False
    assert result["max_commits"] == 0
