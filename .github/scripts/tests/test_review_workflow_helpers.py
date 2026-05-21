"""Regression tests for review workflow shell helpers."""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import prepare_pr_context

REPO_ROOT = Path(__file__).resolve().parents[3]
SCRIPTS = REPO_ROOT / ".github" / "scripts"


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout


class ResolvePrMetadataTest(unittest.TestCase):
    def run_helper(self, tmp: Path, payload: dict) -> subprocess.CompletedProcess[str]:
        bin_dir = tmp / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            "test \"$1\" = api\n"
            "test \"$2\" = repos/DongwonTTuna/demo/pulls/7\n"
            f"printf '%s\\n' '{json.dumps(payload)}'\n",
            encoding="utf-8",
        )
        gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
        out_file = tmp / "outputs"
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{bin_dir}:{env['PATH']}",
                "REPO": "DongwonTTuna/demo",
                "PR_NUMBER": "7",
                "GITHUB_OUTPUT": str(out_file),
            }
        )
        return subprocess.run(
            ["bash", str(SCRIPTS / "resolve_pr_metadata.sh")],
            env=env,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def test_uses_single_rest_payload_for_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            payload = {
                "head": {
                    "sha": "1" * 40,
                    "repo": {"owner": {"login": "DongwonTTuna"}, "name": "demo"},
                },
                "base": {"ref": "main", "sha": "2" * 40},
                "user": {"login": "DongwonTTuna"},
                "draft": False,
            }
            completed = self.run_helper(tmp, payload)

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                (tmp / "outputs").read_text(encoding="utf-8").splitlines(),
                ["head_sha=" + "1" * 40, "base_ref=main", "base_sha=" + "2" * 40],
            )

    def test_rejects_unsafe_pr_metadata(self) -> None:
        base_payload = {
            "head": {
                "sha": "1" * 40,
                "repo": {"owner": {"login": "DongwonTTuna"}, "name": "demo"},
            },
            "base": {"ref": "main", "sha": "2" * 40},
            "user": {"login": "DongwonTTuna"},
            "draft": False,
        }
        cases = [
            ("draft", {"draft": True}),
            ("author", {"user": {"login": "Other"}}),
            ("fork", {"head": {"repo": {"owner": {"login": "Other"}, "name": "demo"}}}),
            ("non_main", {"base": {"ref": "develop"}}),
            ("bad_head_sha", {"head": {"sha": "not-a-sha"}}),
            ("bad_base_sha", {"base": {"sha": "not-a-sha"}}),
        ]
        for _name, patch_payload in cases:
            payload = json.loads(json.dumps(base_payload))
            for key, value in patch_payload.items():
                if isinstance(value, dict) and isinstance(payload.get(key), dict):
                    payload[key].update(value)
                else:
                    payload[key] = value
            with self.subTest(_name), tempfile.TemporaryDirectory() as td:
                tmp = Path(td)
                completed = self.run_helper(tmp, payload)

                self.assertNotEqual(completed.returncode, 0)
                self.assertFalse((tmp / "outputs").exists())
                self.assertIn("::", completed.stdout)


class PreparePrContextSnapshotTest(unittest.TestCase):
    def test_rejects_head_sha_drift_before_fetching_files(self) -> None:
        env = {
            "GITHUB_REPOSITORY": "DongwonTTuna/demo",
            "PR_NUMBER": "7",
            "HEAD_SHA": "1" * 40,
            "BASE_SHA": "2" * 40,
            "RUNNER_TEMP": tempfile.mkdtemp(),
        }
        with patch.dict(os.environ, env, clear=False), patch.object(
            prepare_pr_context,
            "gh_json",
            return_value={"head": {"sha": "3" * 40}, "base": {"sha": "2" * 40}},
        ), patch.object(prepare_pr_context, "gh_paginated") as gh_paginated:
            with self.assertRaises(SystemExit):
                prepare_pr_context.main()
            gh_paginated.assert_not_called()

    def test_rejects_base_sha_drift_before_fetching_files(self) -> None:
        env = {
            "GITHUB_REPOSITORY": "DongwonTTuna/demo",
            "PR_NUMBER": "7",
            "HEAD_SHA": "1" * 40,
            "BASE_SHA": "2" * 40,
            "RUNNER_TEMP": tempfile.mkdtemp(),
        }
        with patch.dict(os.environ, env, clear=False), patch.object(
            prepare_pr_context,
            "gh_json",
            return_value={"head": {"sha": "1" * 40}, "base": {"sha": "3" * 40}},
        ), patch.object(prepare_pr_context, "gh_paginated") as gh_paginated:
            with self.assertRaises(SystemExit):
                prepare_pr_context.main()
            gh_paginated.assert_not_called()

    def test_rejects_snapshot_drift_after_fetching_files(self) -> None:
        env = {
            "GITHUB_REPOSITORY": "DongwonTTuna/demo",
            "PR_NUMBER": "7",
            "HEAD_SHA": "1" * 40,
            "BASE_SHA": "2" * 40,
            "RUNNER_TEMP": tempfile.mkdtemp(),
        }
        snapshots = [
            {"head": {"sha": "1" * 40}, "base": {"sha": "2" * 40}},
            {"head": {"sha": "3" * 40}, "base": {"sha": "2" * 40}},
        ]
        with patch.dict(os.environ, env, clear=False), patch.object(
            prepare_pr_context,
            "gh_json",
            side_effect=snapshots,
        ), patch.object(
            prepare_pr_context,
            "gh_paginated",
            side_effect=[[], []],
        ) as gh_paginated:
            with self.assertRaises(SystemExit):
                prepare_pr_context.main()
            self.assertEqual(gh_paginated.call_count, 2)

    def test_writes_base_sha_in_context(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            env = {
                "GITHUB_REPOSITORY": "DongwonTTuna/demo",
                "PR_NUMBER": "7",
                "HEAD_SHA": "1" * 40,
                "BASE_SHA": "2" * 40,
                "RUNNER_TEMP": td,
            }
            with patch.dict(os.environ, env, clear=False), patch.object(
                prepare_pr_context,
                "gh_json",
                return_value={"head": {"sha": "1" * 40}, "base": {"sha": "2" * 40}},
            ), patch.object(prepare_pr_context, "gh_paginated", side_effect=[[], []]):
                prepare_pr_context.main()

            context = json.loads(
                (Path(td) / "codex-review-context.json").read_text(encoding="utf-8")
            )
            self.assertEqual(context["pull_request"]["base_sha"], "2" * 40)


class PrepareReviewWorkspaceTest(unittest.TestCase):
    def test_deepens_base_and_head_then_hydrates_final_diff(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            remote = tmp / "remote.git"
            source = tmp / "source"
            clone = tmp / "clone"

            run(["git", "init", "-q", "--bare", str(remote)], tmp)
            run(["git", "config", "uploadpack.allowFilter", "true"], remote)
            run(["git", "config", "uploadpack.allowAnySHA1InWant", "true"], remote)
            source.mkdir()
            run(["git", "init", "-q", "-b", "main"], source)
            run(["git", "config", "user.email", "test@example.com"], source)
            run(["git", "config", "user.name", "Test"], source)
            (source / "file.txt").write_text("base\n", encoding="utf-8")
            run(["git", "add", "file.txt"], source)
            run(["git", "commit", "-q", "-m", "base"], source)
            branch_point_sha = run(["git", "rev-parse", "HEAD"], source).strip()
            run(["git", "checkout", "-q", "-b", "feature"], source)
            (source / "file.txt").write_text("one\n", encoding="utf-8")
            run(["git", "commit", "-qam", "one"], source)
            (source / "file.txt").write_text("two\n", encoding="utf-8")
            run(["git", "commit", "-qam", "two"], source)
            head_sha = run(["git", "rev-parse", "HEAD"], source).strip()
            run(["git", "checkout", "-q", "main"], source)
            (source / "base.txt").write_text("base advanced\n", encoding="utf-8")
            run(["git", "add", "base.txt"], source)
            run(["git", "commit", "-q", "-m", "base advanced"], source)
            base_sha = run(["git", "rev-parse", "HEAD"], source).strip()
            run(["git", "remote", "add", "origin", str(remote)], source)
            run(["git", "push", "-q", "origin", "main", "feature"], source)
            run(
                [
                    "git",
                    "clone",
                    "-q",
                    "--filter=blob:none",
                    "--depth=1",
                    "--branch",
                    "feature",
                    f"file://{remote}",
                    str(clone),
                ],
                tmp,
            )
            self.assertNotEqual(
                subprocess.run(
                    ["git", "merge-base", "HEAD", base_sha],
                    cwd=clone,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    check=False,
                ).returncode,
                0,
            )
            self.assertEqual(
                run(["git", "config", "--get", "remote.origin.promisor"], clone).strip(),
                "true",
            )

            env = os.environ.copy()
            env.update(
                {
                    "BASE_REF": "main",
                    "BASE_SHA": base_sha,
                    "HEAD_SHA": head_sha,
                    "PR_NUMBER": "7",
                    "GIT_AUTH_TOKEN": "test-token",
                    "DEEPEN_STEP": "1",
                    "MAX_DEEPEN_ROUNDS": "8",
                }
            )
            subprocess.run(
                ["bash", str(SCRIPTS / "prepare_review_workspace.sh")],
                cwd=clone,
                env=env,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertEqual(
                run(["git", "merge-base", "refs/remotes/origin/main", "HEAD"], clone).strip(),
                branch_point_sha,
            )
            run(
                [
                    "env",
                    "GIT_NO_LAZY_FETCH=1",
                    "git",
                    "diff",
                    "--no-ext-diff",
                    "refs/remotes/origin/main...HEAD",
                ],
                clone,
            )
            self.assertNotEqual(
                subprocess.run(
                    [
                        "git",
                        "config",
                        "--local",
                        "--name-only",
                        "--get-regexp",
                        r"^(http\..*\.extraheader|credential\.helper)$",
                    ],
                    cwd=clone,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    check=False,
                ).returncode,
                0,
            )

    def test_rejects_invalid_inputs_before_fetching(self) -> None:
        cases = [
            ("../main", "2" * 40, "1" * 40, "7", "Invalid base ref name"),
            ("main", "bad", "1" * 40, "7", "Invalid base sha format"),
            ("main", "2" * 40, "bad", "7", "Invalid head sha format"),
            ("main", "2" * 40, "1" * 40, "abc", "Invalid PR number"),
        ]
        for base_ref, base_sha, head_sha, pr_number, expected in cases:
            with self.subTest(expected), tempfile.TemporaryDirectory() as td:
                tmp = Path(td)
                run(["git", "init", "-q"], tmp)
                env = os.environ.copy()
                env.update(
                    {
                        "BASE_REF": base_ref,
                        "BASE_SHA": base_sha,
                        "HEAD_SHA": head_sha,
                        "PR_NUMBER": pr_number,
                    }
                )
                completed = subprocess.run(
                    ["bash", str(SCRIPTS / "prepare_review_workspace.sh")],
                    cwd=tmp,
                    env=env,
                    check=False,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn(expected, completed.stdout)

    def test_rejects_checked_out_head_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            remote = tmp / "remote.git"
            source = tmp / "source"
            clone = tmp / "clone"

            run(["git", "init", "-q", "--bare", str(remote)], tmp)
            source.mkdir()
            run(["git", "init", "-q", "-b", "main"], source)
            run(["git", "config", "user.email", "test@example.com"], source)
            run(["git", "config", "user.name", "Test"], source)
            (source / "file.txt").write_text("base\n", encoding="utf-8")
            run(["git", "add", "file.txt"], source)
            run(["git", "commit", "-q", "-m", "base"], source)
            base_sha = run(["git", "rev-parse", "HEAD"], source).strip()
            run(["git", "checkout", "-q", "-b", "feature"], source)
            (source / "file.txt").write_text("head\n", encoding="utf-8")
            run(["git", "commit", "-qam", "head"], source)
            head_sha = run(["git", "rev-parse", "HEAD"], source).strip()
            run(["git", "remote", "add", "origin", str(remote)], source)
            run(["git", "push", "-q", "origin", "main", "feature"], source)
            run(
                [
                    "git",
                    "clone",
                    "-q",
                    "--depth=1",
                    "--branch",
                    "main",
                    f"file://{remote}",
                    str(clone),
                ],
                tmp,
            )
            env = os.environ.copy()
            env.update(
                {
                    "BASE_REF": "main",
                    "BASE_SHA": base_sha,
                    "HEAD_SHA": head_sha,
                    "PR_NUMBER": "7",
                }
            )

            completed = subprocess.run(
                ["bash", str(SCRIPTS / "prepare_review_workspace.sh")],
                cwd=clone,
                env=env,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("Checked out head does not match", completed.stdout)

    def test_fails_when_merge_base_stays_out_of_range(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            remote = tmp / "remote.git"
            base = tmp / "base"
            feature = tmp / "feature"
            clone = tmp / "clone"

            run(["git", "init", "-q", "--bare", str(remote)], tmp)
            for repo in (base, feature):
                repo.mkdir()
                run(["git", "init", "-q", "-b", "main"], repo)
                run(["git", "config", "user.email", "test@example.com"], repo)
                run(["git", "config", "user.name", "Test"], repo)
                (repo / "file.txt").write_text(repo.name + "\n", encoding="utf-8")
                run(["git", "add", "file.txt"], repo)
                run(["git", "commit", "-q", "-m", repo.name], repo)
            base_sha = run(["git", "rev-parse", "HEAD"], base).strip()
            run(["git", "remote", "add", "origin", str(remote)], base)
            run(["git", "push", "-q", "origin", "main"], base)
            run(["git", "checkout", "-q", "-b", "feature"], feature)
            head_sha = run(["git", "rev-parse", "HEAD"], feature).strip()
            run(["git", "remote", "add", "origin", str(remote)], feature)
            run(["git", "push", "-q", "origin", "feature"], feature)
            run(
                [
                    "git",
                    "clone",
                    "-q",
                    "--depth=1",
                    "--branch",
                    "feature",
                    f"file://{remote}",
                    str(clone),
                ],
                tmp,
            )
            env = os.environ.copy()
            env.update(
                {
                    "BASE_REF": "main",
                    "BASE_SHA": base_sha,
                    "HEAD_SHA": head_sha,
                    "PR_NUMBER": "7",
                    "DEEPEN_STEP": "1",
                    "MAX_DEEPEN_ROUNDS": "1",
                }
            )

            completed = subprocess.run(
                ["bash", str(SCRIPTS / "prepare_review_workspace.sh")],
                cwd=clone,
                env=env,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("Could not find merge-base", completed.stdout)


class ReviewWorkflowBootstrapTest(unittest.TestCase):
    def test_bootstrap_does_not_execute_pr_head_helper(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "codex-pr-review-pipeline.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("Bootstrap path for the PR that introduces the base-ref helper", workflow)
        self.assertNotIn('helper=".github/scripts/prepare_review_workspace.sh"', workflow)
        self.assertEqual(workflow.count("- name: Prepare review git context"), 2)


if __name__ == "__main__":
    unittest.main()
