# Release provenance and dependency risk record

This record describes the PBRSDK-28 repository and dependency snapshot. It is
an audit input, not a general assurance statement.

## Upstream and fork identity

- Upstream repository: `OrderBookTrade/rs-builder-relayer-client`
- Recorded upstream HEAD: `a2306c8`
- Recorded merge-base: `521ab0b`
- Recorded fork difference: 133 files, +33,880 / -1,319
- Upstream original author from the `Cargo.toml` `authors` field: `baice <libaice147@gmail.com>`
- Fork organization: `DongwonTTuna-Labs`

This repository is a derivative work of the upstream repository. `LICENSE-MIT`,
`LICENSE-APACHE`, and `NOTICE` carry the licensing and attribution material
without inventing a copyright year or an additional copyright holder.

## Dependency posture

The `src/direct.rs` HTTP RPC path changes from rustls with webpki-roots to
platform native TLS. Linux uses OpenSSL, Windows uses SChannel, and Apple
platforms use Security Framework. This changes more than trust anchors: TLS
policy, system-proxy integration, error forms, and platform-specific
certificate processing can also change. `ethers/openssl` is forwarded to
`ethers-middleware`, so the backend changes both the direct `Provider<Http>`
path and middleware-internal reqwest 0.11 use.

This path fails in an environment without a system CA bundle. The crate's main
relayer path already uses platform native TLS through reqwest 0.12, so this is
not a new constraint for most consumers. It is a newly introduced prerequisite
for a consumer that uses only `DirectExecutor`.

In the measured Linux graph, `openssl-sys` already existed through the
reqwest 0.12 to native-tls path, so it is not a new system dependency. On
Windows and Apple platforms, both the existing path and the adopted path use
the platform native TLS implementation, so `openssl-sys` itself does not
apply.

The top-level `ethers/abigen` feature explicitly requested by this crate is
removed because there is no use in `src/` or `examples/`. However,
`ethers-middleware` directly selects `ethers-contract` with
`features = ["abigen", "providers"]`, so this does not guarantee that abigen
components disappear completely from the graph.

The fresh independent resolution contained 640 packages. In that resolution,
reqwest 0.11.27 had `__tls`, `default-tls`, and `native-tls` enabled through
`ethers/openssl`. The unignored `cargo audit --deny warnings` scan no longer
reported RUSTSEC-2026-0098, RUSTSEC-2026-0099, or RUSTSEC-2026-0104; those
removed rustls-webpki advisories are evidence for the dependency change and do
not belong in the accepted-advisory register below.

Snapshot environment:

- Measurement time: `2026-08-02T18:37:36Z`
- `rustc 1.95.0 (59807616e 2026-04-14)`
- `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`
- Target triple: `x86_64-unknown-linux-gnu`
- `cargo-audit-audit 0.22.2`
- Advisory DB commit: `d91a8fc9492378f23cba86b81770c6d16de6ebba`
- Advisory DB update time: `2026-08-02T19:56:20+02:00`

These numbers are a snapshot of an independent resolution at a particular
time. Another dependency in a consumer workspace can reactivate
`ethers/rustls`; Cargo features are additive. This record therefore does not
guarantee the dependency graph of every consumer.

## Accepted advisories

The canonical register is `docs/accepted-advisories.toml`. The table below is
an exact rendering of that machine-readable file, not the source from which the
register is derived.

| Advisory | Crate | Path | Scope | Rationale | Re-review condition |
| --- | --- | --- | --- | --- | --- |
| RUSTSEC-2025-0009 | ring 0.16.20 | ethers 2.0.14 → ethers-providers 2.0.14 → jsonwebtoken 8.3.0 → ring 0.16.20 | shipped | Legacy transitive provider dependency; this crate does not directly call jsonwebtoken or ring, and transitive reachability is not established by this record. | Re-review when ethers, ethers-providers, jsonwebtoken, or ring changes; when direct use is added; or when reachability evidence changes. |
| RUSTSEC-2025-0010 | ring 0.16.20 | ethers 2.0.14 → ethers-providers 2.0.14 → jsonwebtoken 8.3.0 → ring 0.16.20 | shipped | Unmaintained legacy transitive provider dependency retained for the current public compatibility surface. | Re-review when the ethers provider stack is upgraded or replaced, or when ring is used directly. |
| RUSTSEC-2024-0388 | derivative 2.2.0 | polymarket-client-sdk 0.4.4 (dev) → alloy 1.8.3 → alloy-primitives 1.6.1 → ruint 1.20.0 → optional ark-ff 0.3.0/0.4.2 lock edges → derivative 2.2.0 | dev-only | cargo-audit scans the lock entry even though the optional ark-ff edge is not in the active target graph; the owning root is test-only. | Re-review when polymarket-client-sdk, alloy, ruint, or the consumer lock changes, or if the package enters a shipped graph. |
| RUSTSEC-2025-0057 | fxhash 0.2.1 | ethers 2.0.14 → ethers-providers 2.0.14 → hashers 1.0.1 → fxhash 0.2.1 | shipped | Unmaintained transitive hashing dependency; this crate does not call fxhash directly. | Re-review when ethers-providers or hashers changes, or when fxhash is used directly. |
| RUSTSEC-2024-0384 | instant 0.1.13 | ethers 2.0.14 → ethers-middleware 2.0.14 or ethers-providers 2.0.14 → instant 0.1.13 | shipped | Unmaintained transitive timing dependency retained by the current ethers stack; this crate does not call it directly. | Re-review when ethers middleware/providers changes or when instant is used directly. |
| RUSTSEC-2024-0436 | paste 1.0.15 | polymarket-client-sdk 0.4.4 (dev) → alloy 1.8.3 → alloy-primitives 1.6.1 → paste 1.0.15 | dev-only | Unmaintained proc-macro enters through the test-only official SDK comparison dependency. | Re-review when polymarket-client-sdk or alloy changes, or if paste enters a shipped graph. |
| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 | ethers 2.0.14 → ethers-providers 2.0.14 or ethers-middleware 2.0.14 → reqwest 0.11.27 → rustls-pemfile 1.0.4 | shipped | Unmaintained transitive PEM parser remains in the reqwest 0.11 package graph; this crate does not parse PEM with it directly. | Re-review when ethers or reqwest 0.11 changes, or when PEM parsing becomes direct behavior. |

## What this record does not prove

- This is a point-in-time snapshot. `Cargo.lock` is not tracked, so this
  repository alone cannot reproduce it; the authoritative lock is the
  consumer workspace's lock.
- It does not prove transitive reachability. It confirms only that this crate
  does not directly call `jsonwebtoken` or `ring`.
- The preflight parses `docs/accepted-advisories.toml` as the canonical register
  and compares its ID set with `.cargo/audit.toml` before any repository code
  runs. A library unit test runs before integration tests with the package root
  as its working directory, so an in-Cargo-only check could restore both files
  into agreement after the committed state had already violated the contract.
  The pre-Cargo comparison makes that recovery unable to authorize an ignore.
  The in-Cargo audit instead compares the canonical TOML rows with this parsed
  CommonMark table in exact row order and with all six cell values unchanged.
- The working tree must be exactly the tree that would be committed, and the
  tracked workflow set must be exactly the reviewed one. Every other check here
  reads the working tree, but what GitHub Actions runs, and what a consumer
  pins, is the committed tree. The two can be made to differ in ways that leave
  the working copy looking ordinary, and each one lets a local test run report
  on bytes the release does not carry.
- The authority is `git write-tree`, the tree `git commit` would use, read with
  `GIT_NO_REPLACE_OBJECTS` set, not the index listing. `git replace` installs a
  ref that most commands apply transparently, so `git ls-tree` can be made to
  answer with a reviewed tree while the index builds, and the commit records, a
  different one; replacement refs are not pushed, so a consumer would receive
  the tree that was never read. Any ref under `refs/replace/` is refused
  outright as well, because every other tool in the repository would read the
  substitution. `git add -N ghost` records an index entry that the listing
  reports and the tree omits, so a file that never reaches the commit satisfied
  every comparison keyed on the listing. `write-tree` also refuses an index
  with unmerged entries.
- Content is compared by hashing each file on disk with
  `git hash-object --no-filters` against the object id the tree records. Not
  `git diff`, which honours the index's assume-unchanged and skip-worktree
  flags: one `git update-index` call empties its output while the index holds
  other bytes. Not plain `git hash-object` either, which applies the clean
  filters and end-of-line conversion that attributes select, so one tracked
  `.gitattributes` line marking a file `ident` makes `$Id: anything $` on disk
  hash to the same object as `$Id$` in the tree.
- Every path in the tree must be a regular file, and its executable bit must
  match the recorded mode. Mode 160000 is a gitlink: replacing `.github` with
  one leaves a working tree that looks ordinary while the commit holds no
  workflow blobs for Actions to find. Mode 120000 is a symbolic link, whose
  blob is the link target, so the commit would carry that string while an audit
  reading the path on disk got whatever the link pointed at.
  `git update-index --chmod=-x` records 100644 for a file left executable on
  disk, so a script a local run can execute loses its bit in a fresh checkout.
- Every file on disk must be in that tree, apart from a set named in
  `scripts/preflight_build_integrity.py` itself. The set is not read from an
  ignore file: `git ls-files --others` answers from `.gitignore` files found in
  the working tree, tracked or not, so a staged `src/lib.rs` ignore line plus
  `git rm --cached`, or an untracked `src/.gitignore` holding `*`, both empty
  its output while the file stays on disk for every audit to read. Deriving the
  set from the tree fails the same way, because whoever removes every file
  under `src/` also removes `src/` from anything derived from it.
- The nested-repository walk reads the tree rather than asking git what it
  tracks, and it asks no ignore file which directories to skip.
- Whether this is a repository is decided by `git rev-parse --show-toplevel`,
  not by a `.git` entry at the root, and the preflight fails when git cannot
  answer or answers with a different directory. Git metadata can live
  elsewhere: with `GIT_DIR` and `GIT_WORK_TREE` set the repository is fully
  functional and the root holds no `.git` entry, so testing for that entry
  skipped every check above and reported success.
- Every workflow file is pinned by SHA-256 digest in the preflight, and the
  `.github/workflows` directory is closed to its reviewed set. Any workflow in
  this repository can stop the audit from running: one granted
  `permissions: actions: write` can disable it through the Actions API using the
  automatically provided token, so pinning only the audit workflow would leave
  that door open. The pinning is by digest rather than by reading selected
  lines. Checking chosen lines only binds what was thought of:
  both `run` values can stay verbatim while an `if` or `continue-on-error`
  sibling stops either step from running. Changing the workflow therefore
  requires updating that constant, which is a reviewable edit.
- Agreement between this register and the tool configuration does not prove
  that an advisory is actually present in the current lock, that its recorded
  crate/version/path still matches the current graph, that the same advisory
  has not returned through a new path, or that the acceptance rationale is
  still valid. A cargo-audit ignore is advisory-ID-wide and remains ignored
  when its path or version changes.
- The preflight and in-Cargo audit accept `.cargo/audit.toml` only under a closed
  schema: the root contains only `advisories`, that table contains only
  `ignore`, and `ignore` is a string array. This prevents unregistered severity,
  database, output, or warning policy from changing the audit result, but it
  does not validate the current advisory paths or rationales.
- The repository `.cargo/` directory is closed to exactly one entry,
  `audit.toml`. In particular, repository-controlled `config.toml` and
  extensionless `config` files cannot redefine `cargo audit` through an alias.
  The required root `rust-toolchain.toml` is also parsed under a closed schema:
  its root contains only `toolchain`, its settings are limited to `channel`,
  `components`, `targets`, and `profile`, and a `path` override is forbidden.
  `channel` is pinned to the version recorded above: a nightly channel unlocks
  unstable manifest features such as `profile-rustflags`, and `-Clinker=` names
  an executable this repository could also provide. `Cargo.toml` is likewise
  closed to the root sections it actually uses, so `cargo-features` and
  `[profile]` tables are rejected rather than each dangerous flag enumerated.
  An extensionless root `rust-toolchain` file is forbidden as well.
- This audit does not inspect Cargo or toolchain configuration outside the
  repository, such as a parent directory or `$CARGO_HOME`. The outer boundary
  is now the `python3` executable selected by `PATH`; that interpreter binary
  and the runner image are not controlled or validated by this repository.
- The build-integrity boundary is checked before the first Cargo invocation by
  `python3 -I scripts/preflight_build_integrity.py`. A Cargo integration test
  cannot bind inputs such as toolchain selection, Cargo configuration, or
  automatic test discovery that decide whether the real Cargo and test binary
  run at all. Python normally places the script directory first on `sys.path`,
  so isolated mode is mandatory to prevent a repository `scripts/tomllib.py`
  from shadowing the standard library before the checks start. The preflight
  independently closes `scripts/` to its own file, uses Python `tomllib`, and
  reads files, and asks `git` for what the repository records. It also binds
  the committed accepted-advisory
  register, closed audit configuration, both exact workflow run commands,
  manifest publication and ethers TLS fields, and the three non-empty license
  files before Cargo starts. Moving these load-bearing checks outside Cargo
  prevents a unit test from repairing a bad committed input before the
  integration-test audit reads it.
- Every audited path is checked for being a symbolic link before it is read.
  Reading a path follows links, so a linked `.github` would let a local audit
  see a valid workflow while GitHub Actions, which does not treat a linked
  `.github` as a workflow directory, ran nothing at all.
- Build scripts are forbidden: no `build.rs`, and `package.build` and
  `package.metabuild` must be absent or false. A build script runs before any
  test binary is compiled, with the package root as its working directory, so
  it could rewrite the accepted-advisory register and let the in-Cargo audit
  read a document that was never committed. Local path dependencies, `[patch]`,
  `[replace]` and `[workspace]` are forbidden for the same reason: each brings a
  manifest and a build script this audit does not read. The dependency check
  covers the dependency tables both at the top level and under every
  `[target.<cfg>]` section, which is a second home for the same declaration, and
  reads the underscore spellings (`dev_dependencies`, `build_dependencies`)
  that Cargo accepts as distinct keys. `package.workspace` is forbidden too: it
  names another workspace root, which need not be a parent directory.
  Each dependency entry is restricted to the keys a reviewed registry release
  needs -- `version`, `features`, `optional`, `default-features`, `package`.
  Naming the forbidden source keys never finished: `path`, then `git`, then
  `registry-index`, each able to reach repository code. Stating what a
  dependency may carry rejects the forms nobody enumerated.
- The preflight cannot check whether it is itself a symbolic link, because
  Python has already opened and executed the link target before any check in it
  runs. That one check therefore lives in the workflow command, which this
  audit pins exactly. `test -L` inspects only the final path component, so the
  command checks the `scripts` directory as well as the script: a linked
  directory would otherwise leave the leaf a regular file and pass.
- The audit command discards any existing `Cargo.lock` first. `cargo audit`
  reuses a lockfile without comparing it to the manifest, so a stale lock
  committed alongside a new dependency would be audited in its place.
- Cargo auto-discovery flags must be absent or true, and this audit source must
  remain present. If `[[test]]` declarations exist, exactly one may name
  `release_provenance_test`; its path is fixed, `test` and `harness` must be
  absent or true, `required-features` must be absent or empty, and no other
  target keys are accepted. These fields change whether the target participates
  in the default `cargo test` gate even when a direct `--test` invocation works.
- The manifest audit proves only that this manifest explicitly names at least
  one TLS backend for `ethers`. It does not prove that rustls disappears from
  the resolved graph, that exactly one TLS implementation exists, that
  consumer feature unification cannot reactivate rustls, or that an actual
  HTTPS connection succeeds. Cargo features are additive.
- The provenance audit parses this document once with `pulldown-cmark` and
  uses that single CommonMark view for H2 sections, the accepted-advisory
  table, and rendered-text claim checks. It collects IDs only from the one
  parsed table in the parsed `Accepted advisories` H2 section. Pipe-shaped
  text in fenced code is not a table row. It compares that table with the
  canonical TOML register by ID, row order, and every cell value. This proves
  the rendering equality and required cell shape under that parser version; it
  does not prove that a rationale is factually correct or remains current.
- This audit reads `Cargo.toml` through a TOML parser, the security-audit
  workflow through a YAML parser, and this provenance record through a
  CommonMark parser. The `rust-validation.yml` contract checked by
  `tests/ci_contract_test.rs` still uses partial-string matching and remains
  outside this ticket's scope.
- This document is restricted to a narrow Markdown subset and a closed
  character alphabet: printable ASCII U+0020 through U+007E, line feed U+000A,
  and rightwards arrow U+2192 (`→`). Tabs, carriage returns, every other Unicode
  character, code blocks, images, links, raw HTML, and other unsupported
  constructs are rejected rather than interpreted. The character allowlist is
  applied both to the raw Markdown and to every text or code event rendered by
  the CommonMark parser, so an ASCII numeric character reference cannot decode
  to an unreviewed character. A rejected character is never removed or used to
  normalize a heading, advisory cell, or claim. This restriction does not
  establish how another Markdown renderer would interpret a document outside
  the accepted subset.
- The former invisible-character denylist was not a complete defense because
  visual blankness is not a Unicode property. Successive counterexamples
  U+200B, U+2063, and U+2800 crossed the earlier classifications. The closed
  allowlist instead makes every future or unreviewed character disallowed by
  default; adding a character requires an explicit audited policy change.
- This record makes no assertion of vulnerability absence. Any such positive
  claim is outside what it proves.

## Publishing decision

`Cargo.toml` sets `publish = false`. This organization-controlled derivative
is consumed by path dependency during development and by audited commit pin in
production; it is not published to crates.io.

The security-audit workflow runs only on its schedule or by manual dispatch; it
is not a pull-request gate. Consequently, a dependency newly introduced by a
pull request is detected by this workflow only on the next scheduled run or a
manual run against trusted repository content. Pull-request validation must
not treat this workflow as contemporaneous audit evidence.

The workflow does not reference a repository secret, but `actions/checkout`
receives the implicit `GITHUB_TOKEN`. It therefore sets
`persist-credentials: false` so checkout does not leave that token in local git
configuration, and it has no pull-request trigger that would run
contributor-controlled Cargo configuration in the token-bearing job. Scheduled
and trusted manual runs execute `cargo audit --deny warnings`, so new
unmaintained notices fail unless they are explicitly reviewed and added to both
the canonical TOML register and `.cargo/audit.toml`.

The workflow schema fixes four steps exactly: checkout, the Python preflight,
tool installation, and cargo-audit. `actions/checkout` is pinned to commit
`11d5960a326750d5838078e36cf38b85af677262`, and
`taiki-e/install-action` is pinned to commit
`67729d5c413db75907f0ad1e39bb04b9c868ff60`; moving tags and additional actions
are rejected. Both run commands are exact, the preflight must precede tool
installation, no step may use a condition or continue-on-error behavior, and
token-context references are rejected across all parsed YAML scalars.

Scheduled execution uses the default branch's reviewed workflow and preflight,
and there is no pull-request trigger, so a pull request cannot replace either
file in this job before merge. A local gate has a weaker procedural boundary:
the operator must run `python3 -I scripts/preflight_build_integrity.py` before any
Cargo command. This repository cannot mechanically force that local ordering.

The gate deliberately does not prefix every Cargo command with
`rustup run 1.95.0`. Once the outside-Cargo preflight has rejected a toolchain
`path` override, ordinary Cargo resolves the validated `rust-toolchain.toml`.
An explicit toolchain prefix would duplicate that check across every command
while adding documentation and operator friction; it is not part of this
contract.

## Consumer pinning and rollback

Production consumers must pin an audited commit SHA rather than a branch.
Development may use a local path dependency. The consumer workspace owns the
authoritative lock and must run the same audit against that lock before import.

Rollback means restoring the previous audited commit SHA and the matching
consumer lock, rebuilding, and re-running consumer validation. A rollback must
not silently switch to a floating branch or reuse a lock resolved for another
source revision.
