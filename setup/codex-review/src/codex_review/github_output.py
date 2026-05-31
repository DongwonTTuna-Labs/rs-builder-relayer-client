"""GitHub Actions output file helpers."""

from __future__ import annotations

from pathlib import Path


def append_outputs(path: str | Path, values: dict[str, str]) -> None:
    with Path(path).open("a", encoding="utf-8") as output:
        for key, value in values.items():
            print(f"{key}={value}", file=output)
