"""Maps each CLI area to its handler and whether it needs the loaded config."""
from __future__ import annotations

from codex_review.cli.handlers import (
    auth,
    context,
    event,
    io,
    loop,
    oidc,
    schema,
    stage00,
    stage01,
    stage02,
    stage03,
    stage04,
    stage05,
    stage06,
    stage07,
    stage08,
    stage09,
)

AREAS = [
    "auth", "oidc", "io", "event", "context", "loop", "schema",
    "stage00", "stage01", "stage02", "stage03", "stage04",
    "stage05", "stage06", "stage07", "stage08", "stage09",
]

HANDLERS = {
    "auth": auth.handle_auth,
    "oidc": oidc.handle_oidc,
    "io": io.handle_io,
    "event": event.handle_event,
    "context": context.handle_context,
    "loop": loop.handle_loop,
    "schema": schema.handle_schema,
    "stage00": stage00.handle_stage00,
    "stage01": stage01.handle_stage01,
    "stage02": stage02.handle_stage02,
    "stage03": stage03.handle_stage03,
    "stage04": stage04.handle_stage04,
    "stage05": stage05.handle_stage05,
    "stage06": stage06.handle_stage06,
    "stage07": stage07.handle_stage07,
    "stage08": stage08.handle_stage08,
    "stage09": stage09.handle_stage09,
}

# Areas whose handler also receives the loaded config.yml.
NEEDS_CONFIG = {
    "context", "stage00", "stage01", "stage02", "stage03", "stage04",
    "stage05", "stage06", "stage07", "stage08", "stage09",
}
