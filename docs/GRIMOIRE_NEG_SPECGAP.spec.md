# Grimoire Negative Spec-Gap Fixture

## Purpose
This tracked directive is a negative-path smoke fixture for the Grimoire reusable control plane. It asks for a small in-scope documentation change while deliberately omitting the OpenSpec requirement and scenario evidence that would make the change safe to implement.

## Requested Change
Create `docs/GRIMOIRE_NEG_SPECGAP.md` with exactly this marker line:

```text
Grimoire negative spec-gap halt marker.
```

## Expected Grimoire Behavior
Treat the requested docs-only addition as in scope for review/design, then halt at spec-gap because this pull request intentionally does not include a satisfying OpenSpec change requirement or scenario for the requested marker file.

## Forbidden Changes
Do not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, live-capable venue behavior, protected paths, OpenSpec specs, or any file other than the requested marker file. The marker file is intentionally absent before Grimoire runs.
