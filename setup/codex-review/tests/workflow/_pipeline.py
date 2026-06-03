"""Shared helpers for Codex pipeline workflow-shape tests.

The pipeline was split from a single orchestrator into label-driven workflows
(review -> design -> fix -> issue). Security/shape invariants are enforced
across whichever pipeline files currently exist, so these helpers aggregate
across all of them while still allowing per-file structural assertions.
"""
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS_DIR = ROOT / ".github" / "workflows"
CODEX_ACTION = "openai/codex-action@e0fdf01220eb9a88167c4898839d273e3f2609d1"
RESPONSES_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
OIDC_MINT_COMMAND = "codex-review oidc relay-token"

# Files that together implement the Codex review pipeline. Listed in pipeline
# order; not all exist at every step of the split migration.
PIPELINE_FILENAMES = [
    "codex-review.yml",
    "codex-design.yml",
    "codex-fix.yml",
    "codex-issue.yml",
    "codex-review-orchestrator.yml",
]


SETUP_ACTION_PATH = ROOT / ".github" / "actions" / "setup-codex-review" / "action.yml"
SETUP_ACTION_USES = "DongwonTTuna-Labs/rs-builder-relayer-client/.github/actions/setup-codex-review@main"


def setup_action_text() -> str:
    return SETUP_ACTION_PATH.read_text(encoding="utf-8")


def setup_action_steps() -> list[dict]:
    return (yaml.safe_load(setup_action_text()).get("runs") or {}).get("steps") or []


def workflow_path(name: str) -> Path:
    return WORKFLOWS_DIR / name


def exists(name: str) -> bool:
    return workflow_path(name).exists()


def existing_pipeline_files() -> list[Path]:
    return [workflow_path(n) for n in PIPELINE_FILENAMES if exists(n)]


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def jobs_of(name: str) -> dict:
    return load(workflow_path(name)).get("jobs") or {}


def all_jobs() -> dict:
    """Merge jobs across every existing pipeline file (names are unique)."""
    merged: dict = {}
    for path in existing_pipeline_files():
        for job_name, job in (load(path).get("jobs") or {}).items():
            merged[job_name] = job
    return merged


def all_text() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in existing_pipeline_files())


def iter_all_steps():
    for path in existing_pipeline_files():
        for job_name, job in (load(path).get("jobs") or {}).items():
            for step in job.get("steps", []) or []:
                yield job_name, step


def codex_action_steps():
    return [(n, s) for n, s in iter_all_steps() if s.get("uses") == CODEX_ACTION]
