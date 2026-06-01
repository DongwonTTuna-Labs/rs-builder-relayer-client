import json
import os
import subprocess
import textwrap


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../.."))
RUNNER = os.path.join(ROOT, "setup", "codex-review", "bin", "codex-review-model-runner")


def test_model_runner_invokes_codex_and_writes_json(tmp_path):
    fake_codex = tmp_path / "fake-codex"
    fake_codex.write_text(
        textwrap.dedent(
            """\
            #!/usr/bin/env bash
            set -euo pipefail
            output=""
            while [ "$#" -gt 0 ]; do
              if [ "$1" = "--output-last-message" ]; then
                shift
                output="$1"
              fi
              shift || true
            done
            test -n "$output"
            printf '%s' '{"schema_version":"stage00-lifecycle-result.v1","decisions":[]}' > "$output"
            """
        ),
        encoding="utf-8",
    )
    fake_codex.chmod(0o755)

    prompt = tmp_path / "prompt.md"
    prompt.write_text("Return stage00 JSON.\n", encoding="utf-8")
    output = tmp_path / "out.json"

    env = {
        **os.environ,
        "CODEX_REVIEW_CODEX_BIN": str(fake_codex),
        "CODEX_REVIEW_PROMPT_PATH": str(prompt),
        "CODEX_REVIEW_OUTPUT_PATH": str(output),
        "CODEX_REVIEW_EXPECTED_SCHEMA": "stage00-lifecycle-result.v1",
        "CODEX_REVIEW_CODEX_ARGS_JSON": '["--ignore-user-config","--color","never"]',
        "AI_RELAY_API_KEY": "sk-clb-test",
        "CODEX_REVIEW_MODEL_CWD": ROOT,
    }

    subprocess.run([RUNNER], check=True, cwd=ROOT, env=env)

    assert json.loads(output.read_text(encoding="utf-8")) == {
        "schema_version": "stage00-lifecycle-result.v1",
        "decisions": [],
    }


def test_model_runner_does_not_pass_removed_approval_flag(tmp_path):
    fake_codex = tmp_path / "fake-codex"
    fake_codex.write_text(
        textwrap.dedent(
            """\
            #!/usr/bin/env bash
            set -euo pipefail
            if [ "${1:-}" = "exec" ] && [ "${2:-}" = "--help" ]; then
              printf '%s\n' 'Usage: codex exec [OPTIONS] [PROMPT]'
              exit 0
            fi
            output=""
            while [ "$#" -gt 0 ]; do
              if [ "$1" = "--ask-for-approval" ]; then
                echo "unexpected argument '--ask-for-approval' found" >&2
                exit 2
              fi
              if [ "$1" = "--output-last-message" ]; then
                shift
                output="$1"
              fi
              shift || true
            done
            test -n "$output"
            printf '%s' '{"schema_version":"stage00-lifecycle-result.v1","decisions":[]}' > "$output"
            """
        ),
        encoding="utf-8",
    )
    fake_codex.chmod(0o755)

    prompt = tmp_path / "prompt.md"
    prompt.write_text("Return stage00 JSON.\n", encoding="utf-8")
    output = tmp_path / "out.json"

    env = {
        **os.environ,
        "CODEX_REVIEW_CODEX_BIN": str(fake_codex),
        "CODEX_REVIEW_PROMPT_PATH": str(prompt),
        "CODEX_REVIEW_OUTPUT_PATH": str(output),
        "CODEX_REVIEW_EXPECTED_SCHEMA": "stage00-lifecycle-result.v1",
        "CODEX_REVIEW_CODEX_ARGS_JSON": '["--ignore-user-config","--color","never"]',
        "AI_RELAY_API_KEY": "sk-clb-test",
        "CODEX_REVIEW_MODEL_CWD": ROOT,
    }

    subprocess.run([RUNNER], check=True, cwd=ROOT, env=env)

    assert json.loads(output.read_text(encoding="utf-8")) == {
        "schema_version": "stage00-lifecycle-result.v1",
        "decisions": [],
    }
