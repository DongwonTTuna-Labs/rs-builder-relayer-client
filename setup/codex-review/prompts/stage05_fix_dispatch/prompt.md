Return exactly one JSON object.
Schema: codex.stage06.fix_outputs.v1.
For every dispatch task, produce exactly one outputs entry.
No markdown, no prose, no code fences, no logs.
Use unified diffs only.
Only modify files listed in allowed_files.
The PR head checkout is in workspace; read target files there and emit repository-relative unified diffs without a workspace/ prefix.
Do not emit an empty outputs array.
If a valid JSON output would be too large or uncertain, emit conflict outputs with empty patch, touched_files, tests, and a concise conflict_reason.
Do not push or post comments.
