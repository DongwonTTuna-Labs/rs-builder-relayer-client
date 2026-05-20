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
    def test_uses_single_rest_payload_for_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            bin_dir = tmp / "bin"
            bin_dir.mkdir()
            gh = bin_dir / "gh"
            payload = {
                "head": {
                    "sha": "1" * 40,
                    "repo": {"owner": {"login": "DongwonTTuna"}, "name": "demo"},
                },
                "base": {"ref": "main", "sha": "2" * 40},
                "draft": False,
            }
            gh.write_text(
                "#!/usr/bin/env bash\n"
                "test \"$1\" = api\n"
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

            subprocess.run(
                ["bash", str(SCRIPTS / "resolve_pr_metadata.sh")],
                env=env,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertEqual(
                out_file.read_text(encoding="utf-8").splitlines(),
                ["head_sha=" + "1" * 40, "base_ref=main", "base_sha=" + "2" * 40],
            )


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


class PrepareReviewWorkspaceTest(unittest.TestCase):
    def test_deepens_shallow_head_and_hydrates_commit_patches(self) -> None:
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
            (source / "file.txt").write_text("one\n", encoding="utf-8")
            run(["git", "commit", "-qam", "one"], source)
            (source / "file.txt").write_text("two\n", encoding="utf-8")
            run(["git", "commit", "-qam", "two"], source)
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

            env = os.environ.copy()
            env.update(
                {
                    "BASE_REF": "main",
                    "BASE_SHA": base_sha,
                    "HEAD_SHA": head_sha,
                    "PR_NUMBER": "7",
                    "DEEPEN_STEP": "1",
                    "MAX_DEEPEN_ROUNDS": "4",
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

            run(["git", "merge-base", "refs/remotes/origin/main", "HEAD"], clone)
            run(
                [
                    "env",
                    "GIT_NO_LAZY_FETCH=1",
                    "git",
                    "show",
                    "--no-ext-diff",
                    "--format=",
                    "HEAD",
                ],
                clone,
            )


if __name__ == "__main__":
    unittest.main()
