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
            "draft": False,
        }
        cases = [
            ("draft", {"draft": True}),
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
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            run(["git", "init", "-q"], tmp)
            env = os.environ.copy()
            env.update(
                {
                    "BASE_REF": "../main",
                    "BASE_SHA": "2" * 40,
                    "HEAD_SHA": "1" * 40,
                    "PR_NUMBER": "7",
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
            self.assertIn("Invalid base ref name", completed.stdout)

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


if __name__ == "__main__":
    unittest.main()
