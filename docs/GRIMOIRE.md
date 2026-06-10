# Grimoire CI Secrets And Model Mapping

This is a stub for Task 3. The full grimoire architecture and operations guide is deferred to Task 18.

## Secrets / Key Sources

- `AI_RELAY_API_KEY`: ai-relay model key. This is dual-source in CI: supplied either as a GitHub Actions secret or inherited from the self-hosted runner Docker environment. The attune/grimoire resolver reads `AI_RELAY_API_KEY_SECRET` from `secrets.AI_RELAY_API_KEY` first, then runner-inherited `AI_RELAY_API_KEY`, masks the selected value, exports it for opencode, and fails closed when neither source is present. The ai-relay `baseURL` is hardcoded in `opencode.json`, so no relay URL secret is needed.
- `GRIMOIRE_PAT`: PAT used for all grimoire GitHub authentication, including checkout, push, `gh`, comments, and labels. Self-hosted runners may provide `CODEX_LOOP_PAT` as a fallback PAT environment variable. The PAT must be repo-scoped and least-privilege.

## PAT-Only GitHub Auth

Grimoire never uses `GITHUB_TOKEN` or a GitHub App token for git, `gh`, checkout, push, comment, or label operations. All GitHub mutation and private checkout paths must use `GRIMOIRE_PAT` first or runner-provided `CODEX_LOOP_PAT` second, then fail closed when neither PAT path is present.

Minimal fine-grained PAT scopes for this repository:

- Contents: read/write.
- Pull requests: read/write.
- Issues: read/write.
- Metadata: read.

The Workflows scope is not needed because the trusted controller halts on `.github/**` changes. For a classic PAT, `repo` alone is sufficient; do not add `workflow` for grimoire.

## Ai-Relay Provider Wiring

Repository CI uses the `ai-relay` provider configured in `opencode.json` with OpenAI-compatible wiring through `@ai-sdk/openai`. The provider reads the key from `{env:AI_RELAY_API_KEY}` after the workflow resolver chooses the GitHub secret or runner env source. The relay endpoint is configured in source as the provider `baseURL`; do not create a separate relay URL secret.

## Model Mapping

All CI agents and categories use `ai-relay/gpt-5.5`. Local Anthropic mappings for `sisyphus` and `prometheus` are replaced in CI by `ai-relay/gpt-5.5` with the `xhigh` variant.

Variant tiers:

- `xhigh`: heavy agents plus `visual-engineering`, `ultrabrain`, `deep`, `artistry`, and `unspecified-high`.
- `medium`: `librarian`, `explore`, `sisyphus-junior`, `quick`, and `unspecified-low`.
- `high`: `writing`.

## Rotation And Least Privilege

Rotate `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, and fallback `CODEX_LOOP_PAT` on the same schedule as other CI credentials, immediately after suspected exposure, and after any runner ownership or repository access change. Keep PAT scopes limited to the grimoire duties above, prefer repo-bound fine-grained tokens, and remove unused fallback paths when they are no longer operationally required.
