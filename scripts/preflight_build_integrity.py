#!/usr/bin/env python3
"""Validate repository-controlled Cargo bootstrap inputs before Cargo runs."""

from __future__ import annotations

import hashlib
import os
import re
import stat
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


REGULAR_FILE_MODES = {"100644", "100755"}

# Everything in the working tree has to be in the commit except these. The set
# is fixed here, in the audited script, rather than read from an ignore file:
# `git ls-files --others` answers from `.gitignore` files found in the working
# tree, tracked or not, so an ignore entry can hide the very file being kept
# out of the release. Deriving the set from the tree fails the same way, since
# whoever removes every file under `src/` also removes `src/` from anything
# derived. A path listed here is one a reviewer can see is not audited, and
# adding to the list is a change to the file this check hashes.
UNCOMMITTED_DIRECTORIES = {
    "target",  # cargo build output
    ".fable-sol",  # working notes kept outside the release
    ".pytest_cache",  # pytest cache
    "setup",  # local tooling checkout, not release content
}
UNCOMMITTED_FILES = {
    "Cargo.lock",  # deliberately untracked; docs/RELEASE_PROVENANCE.md says why
    ".claude/settings.local.json",  # per-developer tool settings
}

# `git replace` installs a ref that most commands apply transparently, so
# `git ls-tree` can be made to answer with a reviewed tree while the index
# builds, and `git commit` records, a different one. Replacement refs are not
# pushed by default, so a consumer would receive the unsubstituted tree.
GIT_ENV = {**os.environ, "GIT_NO_REPLACE_OBJECTS": "1"}

ALLOWED_TOOLCHAIN_KEYS = {"channel", "components", "targets", "profile"}
PINNED_TOOLCHAIN_CHANNEL = "1.95.0"

# `cargo-features` and `[profile]` can carry `rustflags`, and `-Clinker=` names
# an executable this repository could also provide. The manifest is closed to
# the sections it actually uses instead of naming each dangerous one.
ALLOWED_MANIFEST_ROOT_KEYS = {
    "package",
    "lib",
    "example",
    "test",
    "target",
    "features",
    "dependencies",
    "dev-dependencies",
    "dev_dependencies",
    "build-dependencies",
    "build_dependencies",
}
AUTO_DISCOVERY_KEYS = ("autotests", "autobins", "autoexamples", "autobenches")
RELEASE_PROVENANCE_TEST = Path("tests/release_provenance_test.rs")
ACCEPTED_ADVISORIES = Path("docs/accepted-advisories.toml")
SECURITY_AUDIT_WORKFLOW = Path(".github/workflows/security-audit.yml")
# Any workflow in this repository can stop the audit workflow from running:
# one with `permissions: actions: write` can disable it through the Actions API
# using the automatically provided token. Pinning only the audit workflow would
# leave that door open, so the directory is closed to its reviewed set and every
# entry is pinned. Editing any of them requires updating a constant here.
REVIEWED_WORKFLOW_SHA256 = {
    "grimoire.yml": "5374f226020540d95a77a902d684b5b81d91f36c696f9cae9b4bfc9aa86cfcf8",
    "rust-validation.yml": "2580eb2122e481a8804c488925e85de1ba0b7d846bf221a4dc45fcae37397f73",
    "security-audit.yml": "6afa47f1354fa0d5fab4873a31299df6a12f098e27ff536f4ed9d3cdf8410552",
}
PREFLIGHT_COMMAND = (
    "test ! -L scripts && test ! -L scripts/preflight_build_integrity.py && "
    "python3 -I scripts/preflight_build_integrity.py"
)
AUDIT_COMMAND = "rm -f Cargo.lock && cargo audit --deny warnings"
ADVISORY_FIELDS = {"id", "crate", "path", "scope", "rationale", "re-review"}
ADVISORY_SCOPES = {"shipped", "dev-only"}
ALLOWED_AUDIT_TARGET_KEYS = {
    "name",
    "path",
    "test",
    "harness",
    "required-features",
}


def load_toml(path: Path, label: str, errors: list[str]) -> dict[str, Any] | None:
    try:
        with path.open("rb") as source:
            document = tomllib.load(source)
    except (OSError, tomllib.TOMLDecodeError) as error:
        errors.append(f"{label} must be readable valid TOML: {error}")
        return None
    return document


def check_toolchain(root: Path, errors: list[str]) -> None:
    if (root / "rust-toolchain").exists():
        errors.append("rust-toolchain must not exist; use the reviewed TOML file")

    path = root / "rust-toolchain.toml"
    if not path.is_file():
        errors.append("rust-toolchain.toml must exist as a regular file")
        return
    document = load_toml(path, "rust-toolchain.toml", errors)
    if document is None:
        return
    if set(document) != {"toolchain"}:
        errors.append(
            "rust-toolchain.toml root keys must be exactly ['toolchain']; "
            f"found {sorted(document)}"
        )
        return

    toolchain = document["toolchain"]
    if not isinstance(toolchain, dict):
        errors.append("rust-toolchain.toml toolchain must be a table")
        return
    if "path" in toolchain:
        errors.append("rust-toolchain.toml toolchain.path is forbidden")
    # A nightly channel unlocks unstable manifest features such as
    # profile-rustflags, which can name a repository executable as the linker.
    # The channel is therefore pinned to the value this record documents.
    channel = toolchain.get("channel")
    if channel != PINNED_TOOLCHAIN_CHANNEL:
        errors.append(
            f"rust-toolchain.toml toolchain.channel must be "
            f"{PINNED_TOOLCHAIN_CHANNEL!r}; found {channel!r}"
        )
    unsupported = sorted(set(toolchain) - ALLOWED_TOOLCHAIN_KEYS - {"path"})
    if unsupported:
        errors.append(f"rust-toolchain.toml has unsupported toolchain keys: {unsupported}")
    channel = toolchain.get("channel")
    if not isinstance(channel, str) or not channel.strip():
        errors.append("rust-toolchain.toml toolchain.channel must be a non-empty string")


def check_cargo_directory(root: Path, errors: list[str]) -> None:
    cargo_directory = root / ".cargo"
    try:
        entries = sorted(entry.name for entry in cargo_directory.iterdir())
    except OSError as error:
        errors.append(f".cargo must be a readable directory: {error}")
        return
    if entries != ["audit.toml"]:
        errors.append(f".cargo entries must be exactly ['audit.toml']; found {entries}")


def check_scripts_directory(root: Path, errors: list[str]) -> None:
    scripts_directory = root / "scripts"
    try:
        entries = sorted(entry.name for entry in scripts_directory.iterdir())
    except OSError as error:
        errors.append(f"scripts must be a readable directory: {error}")
        return
    if entries != ["preflight_build_integrity.py"]:
        errors.append(
            "scripts entries must be exactly ['preflight_build_integrity.py']; "
            f"found {entries}"
        )


def check_manifest_and_test(root: Path, errors: list[str]) -> None:
    manifest = load_toml(root / "Cargo.toml", "Cargo.toml", errors)
    if manifest is None:
        return

    package = manifest.get("package")
    if not isinstance(package, dict):
        errors.append("Cargo.toml package must be a table")
    else:
        for key in AUTO_DISCOVERY_KEYS:
            if key in package and package[key] is not True:
                errors.append(f"Cargo.toml package.{key} must be absent or true")

    test_path = root / RELEASE_PROVENANCE_TEST
    if not test_path.is_file():
        errors.append(f"{RELEASE_PROVENANCE_TEST} must exist as a regular file")

    if "test" not in manifest:
        return
    declared_tests = manifest["test"]
    if not isinstance(declared_tests, list) or not all(
        isinstance(entry, dict) for entry in declared_tests
    ):
        errors.append("Cargo.toml [[test]] declarations must be an array of tables")
        return
    audit_targets = [
        entry
        for entry in declared_tests
        if entry.get("name") == "release_provenance_test"
    ]
    if len(audit_targets) != 1:
        errors.append(
            "Cargo.toml [[test]] declarations must contain exactly one "
            f"release_provenance_test entry; found {len(audit_targets)}"
        )
        return

    audit_target = audit_targets[0]
    unsupported = sorted(set(audit_target) - ALLOWED_AUDIT_TARGET_KEYS)
    if unsupported:
        errors.append(
            "Cargo.toml release_provenance_test has unsupported keys: "
            f"{unsupported}"
        )
    if audit_target.get("path") != str(RELEASE_PROVENANCE_TEST):
        errors.append(
            "Cargo.toml release_provenance_test path must be exactly "
            "tests/release_provenance_test.rs"
        )
    for key in ("test", "harness"):
        if key in audit_target and audit_target[key] is not True:
            errors.append(
                f"Cargo.toml release_provenance_test {key} must be absent or true"
            )
    required_features = audit_target.get("required-features")
    if required_features is not None and required_features != []:
        errors.append(
            "Cargo.toml release_provenance_test required-features must be absent "
            "or an empty array"
        )


# Every audited path is read through the filesystem, which follows symlinks.
# A repository can therefore make `.github` a link: local reads still see a
# valid workflow, while GitHub Actions declines to treat a linked `.github`
# as a workflow directory and runs nothing. `lstat` is the only way to see
# the link itself, so each audited path is checked before it is read.
# Files whose committed presence the audit depends on.
TRACKED_AUDIT_INPUTS = (
    ".cargo/audit.toml",
    ".github/workflows/grimoire.yml",
    ".github/workflows/rust-validation.yml",
    ".github/workflows/security-audit.yml",
    "Cargo.toml",
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "NOTICE",
    "docs/RELEASE_PROVENANCE.md",
    "docs/accepted-advisories.toml",
    "rust-toolchain.toml",
    "scripts/preflight_build_integrity.py",
    "tests/release_provenance_test.rs",
)

SYMLINK_FORBIDDEN_PATHS = (
    ".github",
    ".github/workflows",
    ".github/workflows/security-audit.yml",
    ".cargo",
    ".cargo/audit.toml",
    "scripts",
    "scripts/preflight_build_integrity.py",
    "Cargo.toml",
    "rust-toolchain.toml",
    "docs",
    "docs/RELEASE_PROVENANCE.md",
    "docs/accepted-advisories.toml",
    "tests",
    "tests/release_provenance_test.rs",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "NOTICE",
)

# Build scripts run before any test binary is compiled, with the package root
# as their working directory. One could rewrite the provenance register so the
# in-Cargo audit reads a document that was never committed. Nothing in this
# repository needs a build script, so their absence is the contract.
BUILD_SCRIPT_KEYS = ("build", "metabuild")


def committed_tree_entries(root: Path, errors: list[str]) -> dict[str, tuple[str, str]] | None:
    """Return (mode, object id) for every path the commit would carry.

    The authority is `git write-tree`, which builds the tree `git commit`
    would use, not `git ls-files --stage`, which lists the index. The two
    differ: `git add -N ghost` records an index entry that the listing shows
    and the tree omits, so a file that never reaches the commit passed every
    comparison keyed on the listing. `write-tree` also refuses an index with
    unmerged entries, which the stage column used to be checked for by hand.

    Asking git does not widen the trust base, because git produced the tree
    being audited in the first place.
    """
    try:
        replacements = subprocess.run(
            ["git", "for-each-ref", "--format=%(refname)", "refs/replace/"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
            env=GIT_ENV,
        ).stdout.split()
        tree = subprocess.run(
            ["git", "write-tree"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
            env=GIT_ENV,
        ).stdout.strip()
        listing = subprocess.run(
            ["git", "ls-tree", "-r", "-z", "--full-tree", tree],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
            env=GIT_ENV,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        errors.append(f"git must produce the candidate commit tree: {error}")
        return None

    # `GIT_NO_REPLACE_OBJECTS` above already makes these calls read the real
    # objects. Their presence is still refused, because every other tool in the
    # repository would read the substitution.
    for refname in sorted(replacements):
        errors.append(
            f"{refname} exists; a replacement ref makes git answer with one tree "
            "while the commit records another"
        )

    entries: dict[str, tuple[str, str]] = {}
    for record in listing.split("\0"):
        if not record:
            continue
        metadata, _, path_name = record.partition("\t")
        fields = metadata.split()
        if len(fields) != 3:
            errors.append(f"git ls-tree produced an unreadable record: {record!r}")
            continue
        mode, _kind, object_id = fields
        entries[path_name] = (mode, object_id)
    return entries


def check_committed_tree(root: Path, errors: list[str]) -> None:
    """Require the working tree to be exactly the tree that would be committed.

    Every other check in this script reads the working tree, but what GitHub
    Actions runs is the committed tree, and so is what a consumer pins. The
    two can be made to differ in ways that leave the working copy looking
    ordinary, and each one lets a local `cargo test` report on bytes the
    release does not carry.
    """
    # Not `(root / ".git").exists()`. Git metadata can live elsewhere: with
    # `GIT_DIR` and `GIT_WORK_TREE` set, the repository is fully functional and
    # the root holds no `.git` entry, so that test skipped every check below
    # and reported success. Ask git where its working tree is instead, and stop
    # with an error when it cannot say.
    try:
        toplevel = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
            env=GIT_ENV,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        errors.append(
            "this preflight must run inside the git working tree it audits, "
            f"because the committed tree is what it binds: {error}"
        )
        return
    if Path(toplevel).resolve() != root.resolve():
        errors.append(
            f"this preflight must run at the git working-tree root; git reports "
            f"{toplevel} and this run is in {root}"
        )
        return

    entries = committed_tree_entries(root, errors)
    if entries is None:
        return

    recorded = {path_name: mode for path_name, (mode, _) in entries.items()}

    # `git rm --cached` removes a file from the tree while leaving it in
    # place, so each audited input has to be confirmed present as regular
    # content.
    for relative in TRACKED_AUDIT_INPUTS:
        mode = recorded.get(relative)
        if mode is None:
            errors.append(
                f"{relative} must be tracked in this repository; "
                "an untracked file is absent from the committed tree"
            )
        elif mode not in REGULAR_FILE_MODES:
            errors.append(
                f"{relative} must be recorded as a regular file; found mode {mode}"
            )

    comparable = []
    for relative in sorted(entries):
        mode, _object_id = entries[relative]
        if mode not in REGULAR_FILE_MODES:
            # Mode 160000 is a gitlink: replacing `.github` with one leaves a
            # working tree that looks ordinary while the commit holds no
            # workflow blobs for Actions to find. Mode 120000 is a symbolic
            # link, whose blob is the link target, so the commit would carry
            # that string while an audit reading the path on disk gets
            # whatever the link points at, anywhere on the machine.
            detail = (
                "recorded as a gitlink"
                if mode == "160000"
                else f"recorded with mode {mode}"
            )
            errors.append(
                f"{relative} is {detail}; every path in the committed tree must be "
                "a regular file, so that what an audit reads on disk is what the "
                "commit carries"
            )
            continue
        path = root / relative
        if path.is_symlink():
            errors.append(
                f"{relative} is committed as a regular file but is a symbolic "
                "link in the working tree"
            )
            continue
        if not path.is_file():
            errors.append(
                f"{relative} is in the committed tree but is not a regular file "
                "in the working tree"
            )
            continue
        # The mode is part of the tree. `git update-index --chmod=-x` records
        # 100644 for a file left executable on disk, so a script a local run
        # can execute loses its bit in a fresh checkout.
        executable = bool(path.stat().st_mode & stat.S_IXUSR)
        if executable != (mode == "100755"):
            errors.append(
                f"{relative} is recorded with mode {mode} but is "
                f"{'executable' if executable else 'not executable'} in the working tree"
            )
            continue
        comparable.append(relative)

    if comparable:
        # `--no-filters` is required, not optional. `git hash-object` applies
        # the clean filters and end-of-line conversion that attributes select,
        # so a single tracked `.gitattributes` line marking a file `ident`
        # makes `$Id: anything $` on disk hash to the same object as `$Id$` in
        # the tree. The comparison has to be over raw bytes, and no external
        # filter program should run during a preflight.
        try:
            hashed = subprocess.run(
                ["git", "hash-object", "--no-filters", "--", *comparable],
                cwd=root,
                capture_output=True,
                text=True,
                check=True,
                env=GIT_ENV,
            ).stdout.split()
        except (OSError, subprocess.CalledProcessError) as error:
            errors.append(f"git hash-object must succeed for committed paths: {error}")
        else:
            if len(hashed) != len(comparable):
                errors.append(
                    "git hash-object must return one hash per committed path; "
                    f"asked for {len(comparable)} and received {len(hashed)}"
                )
            else:
                for relative, digest in zip(comparable, hashed):
                    if digest != entries[relative][1]:
                        errors.append(
                            f"{relative} differs between the candidate commit "
                            "tree and the working tree; the audited bytes must be "
                            "the bytes that get committed"
                        )

    # The comparison above covers only paths the tree holds. A file on disk and
    # absent from the tree is read by every audit that walks the repository and
    # is missing from the release, so the walk decides this, not
    # `git ls-files --others`. That listing answers from `.gitignore` files
    # found in the working tree, tracked or not: a staged `src/lib.rs` ignore
    # line plus `git rm --cached`, or an untracked `src/.gitignore` holding
    # `*`, both empty its output while the file stays on disk.
    #
    for path in sorted(walk_repository_files(root, errors)):
        relative = path.relative_to(root).as_posix()
        if relative in entries:
            continue
        errors.append(
            f"{relative} is not in the committed tree; a file the release does "
            "not carry is still read by everything that walks this repository. "
            f"Commit it, remove it, or name it in {Path(__file__).name}"
        )

    workflow_entries = sorted(
        name for name in entries if name.startswith(".github/workflows/")
    )
    expected_entries = sorted(
        f".github/workflows/{name}" for name in REVIEWED_WORKFLOW_SHA256
    )
    if workflow_entries != expected_entries:
        errors.append(
            f"tracked workflow entries must be exactly {expected_entries}; "
            f"found {workflow_entries}"
        )


def walk_repository_files(root: Path, errors: list[str]) -> list[Path]:
    """Walk the repository, rejecting nested repositories, and return its files.

    A nested `.git` makes a directory a repository of its own, so the parent
    commit can carry none of its contents while the working tree looks whole.
    The directories are walked rather than named. A named set had to be
    extended by hand for every new audited directory, and nothing forced that;
    `src` was never in it even though the boundary, source-matrix, and
    no-CLOB-surface tests read `src/**` from disk.

    Nothing here asks an ignore file what to skip. An ignore file is part of
    the working tree and writable by whoever prepares it, tracked or not, so
    it could name the very directory doing the hiding. The skipped paths are
    fixed below instead, where a reviewer sees them.

    This walk must also hold without an index, so it reads the tree rather
    than asking git what it tracks.
    """
    if (root / ".gitmodules").exists():
        errors.append(".gitmodules is forbidden; an audited path must not be a submodule")

    files: list[Path] = []
    pending = [root]
    while pending:
        current = pending.pop()
        try:
            entries = sorted(current.iterdir())
        except OSError as error:
            errors.append(f"{current} must be a readable directory: {error}")
            continue
        for entry in entries:
            # A submodule records `.git` as a file holding `gitdir: ...`, not
            # as a directory, so the name is checked before anything narrows
            # the entry to directories.
            if entry.name == ".git":
                if current != root:
                    relative = current.relative_to(root).as_posix()
                    errors.append(
                        f"{relative}/.git exists; {relative} must be a directory in this "
                        "repository, not a submodule"
                    )
                continue
            # The kind is settled before the exclusion, so that a name in
            # either set cannot exempt a symbolic link or anything else the
            # set did not mean.
            if entry.is_symlink():
                files.append(entry)
                continue
            relative = entry.relative_to(root).as_posix()
            if entry.is_dir():
                if relative not in UNCOMMITTED_DIRECTORIES:
                    pending.append(entry)
            elif relative not in UNCOMMITTED_FILES:
                files.append(entry)
    return files


def check_no_symlinks(root: Path, errors: list[str]) -> None:
    for relative in SYMLINK_FORBIDDEN_PATHS:
        path = root / relative
        if path.is_symlink():
            errors.append(f"{relative} must be a regular path, not a symlink")


# A local path dependency brings its own manifest and its own `build.rs`, and
# that build script runs on the same terms as the root one. Forbidding the
# dependency form is simpler than auditing every manifest it could reach.
# Cargo accepts the underscore spellings as distinct TOML keys alongside the
# canonical hyphenated ones, so both have to be read.
# A dependency this audit can vouch for is a released crate named by version.
# Anything that selects a source — `path`, `git`, `registry`, `registry-index`
# and their companions — can reach code inside this repository.
ALLOWED_DEPENDENCY_KEYS = {
    "version",
    "features",
    "optional",
    "default-features",
    "default_features",
    "package",
}

DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "dev_dependencies",
    "build-dependencies",
    "build_dependencies",
)


def check_no_local_path_dependencies(root: Path, errors: list[str]) -> None:
    document = load_toml(root / "Cargo.toml", "Cargo.toml", errors)
    if document is None:
        return

    def reject_path_entries(table: Any, label: str) -> None:
        if not isinstance(table, dict):
            return
        for name, spec in table.items():
            if not isinstance(spec, dict):
                continue
            # Naming the forbidden source keys never finished: `path`, then
            # `git`, then `registry-index`, each pointing at repository code.
            # The set of keys a reviewed dependency needs is small and closed,
            # so that is what this states instead.
            unsupported = sorted(set(spec) - ALLOWED_DEPENDENCY_KEYS)
            if unsupported:
                errors.append(
                    f"Cargo.toml {label}.{name} has unsupported dependency keys "
                    f"{unsupported}; only a reviewed registry release may be named"
                )

    for table_name in DEPENDENCY_TABLES:
        reject_path_entries(document.get(table_name), table_name)

    # The same three tables exist again under every `[target.<cfg>]` section,
    # which is a second, easily missed home for a local crate.
    targets = document.get("target")
    if isinstance(targets, dict):
        for triple, target_tables in targets.items():
            if not isinstance(target_tables, dict):
                continue
            for table_name in DEPENDENCY_TABLES:
                reject_path_entries(
                    target_tables.get(table_name), f"target.{triple}.{table_name}"
                )

    for forbidden, reason in (
        ("patch", "it can redirect a crate to local code"),
        ("replace", "it can redirect a crate to local code"),
        ("workspace", "member build scripts are unaudited"),
    ):
        if forbidden in document:
            errors.append(f"Cargo.toml [{forbidden}] is forbidden; {reason}")

    unsupported_root = sorted(set(document) - ALLOWED_MANIFEST_ROOT_KEYS)
    if unsupported_root:
        errors.append(
            f"Cargo.toml has unsupported root sections: {unsupported_root}"
        )

    # `package.workspace` names another workspace root, which need not be a
    # parent directory. That root and its members are outside this audit.
    package = document.get("package")
    if isinstance(package, dict) and "workspace" in package:
        errors.append(
            "Cargo.toml package.workspace is forbidden; it joins an unaudited workspace"
        )


def check_no_build_scripts(root: Path, errors: list[str]) -> None:
    if (root / "build.rs").exists():
        errors.append("build.rs must not exist; build scripts run before the audit")

    document = load_toml(root / "Cargo.toml", "Cargo.toml", errors)
    if document is None:
        return
    package = document.get("package")
    if not isinstance(package, dict):
        return
    for key in BUILD_SCRIPT_KEYS:
        if key in package and package[key] is not False:
            errors.append(
                f"Cargo.toml package.{key} is forbidden unless set to false; "
                f"found {package[key]!r}"
            )


ADVISORY_ID = re.compile(r"^RUSTSEC-\d{4}-\d{4}$")


def check_advisory_register(root: Path, errors: list[str]) -> None:
    """Compare the machine-readable register to the ignore list before Cargo.

    Markdown is deliberately not parsed here. The TOML register is canonical;
    the in-Cargo CommonMark audit separately proves that its human-readable
    rendering has not drifted.
    """
    register = load_toml(
        root / ACCEPTED_ADVISORIES,
        str(ACCEPTED_ADVISORIES),
        errors,
    )
    config = load_toml(root / ".cargo/audit.toml", ".cargo/audit.toml", errors)
    if register is None or config is None:
        return

    if set(register) != {"advisory"}:
        errors.append(
            "docs/accepted-advisories.toml root keys must be exactly "
            f"['advisory']; found {sorted(register)}"
        )
        return
    entries = register.get("advisory")
    if not isinstance(entries, list) or not all(
        isinstance(entry, dict) for entry in entries
    ):
        errors.append("docs/accepted-advisories.toml advisory must be an array of tables")
        return

    registered: list[str] = []
    register_valid = True
    for index, entry in enumerate(entries):
        if set(entry) != ADVISORY_FIELDS:
            errors.append(
                "docs/accepted-advisories.toml advisory "
                f"{index} keys must be exactly {sorted(ADVISORY_FIELDS)}; "
                f"found {sorted(entry)}"
            )
            register_valid = False
        for field in sorted(ADVISORY_FIELDS):
            value = entry.get(field)
            if not isinstance(value, str) or not value.strip():
                errors.append(
                    "docs/accepted-advisories.toml advisory "
                    f"{index}.{field} must be a non-empty string"
                )
                register_valid = False
        advisory_id = entry.get("id")
        if isinstance(advisory_id, str):
            if not ADVISORY_ID.fullmatch(advisory_id):
                errors.append(
                    "docs/accepted-advisories.toml advisory "
                    f"{index}.id has invalid format: {advisory_id!r}"
                )
                register_valid = False
            registered.append(advisory_id)
        scope = entry.get("scope")
        if isinstance(scope, str) and scope not in ADVISORY_SCOPES:
            errors.append(
                "docs/accepted-advisories.toml advisory "
                f"{index}.scope must be shipped or dev-only; found {scope!r}"
            )
            register_valid = False

    duplicates = sorted(
        advisory_id
        for advisory_id in set(registered)
        if registered.count(advisory_id) > 1
    )
    if duplicates:
        errors.append(
            "docs/accepted-advisories.toml has duplicate advisory IDs: "
            f"{duplicates}"
        )
        register_valid = False

    if set(config) != {"advisories"}:
        errors.append(
            ".cargo/audit.toml root keys must be exactly ['advisories']; "
            f"found {sorted(config)}"
        )
        return
    advisories = config.get("advisories")
    if not isinstance(advisories, dict) or set(advisories) != {"ignore"}:
        found = sorted(advisories) if isinstance(advisories, dict) else advisories
        errors.append(
            ".cargo/audit.toml advisories keys must be exactly ['ignore']; "
            f"found {found!r}"
        )
        return
    ignored = advisories.get("ignore")
    if not isinstance(ignored, list) or not all(
        isinstance(advisory_id, str) for advisory_id in ignored
    ):
        errors.append(".cargo/audit.toml advisories.ignore must be a string array")
        return
    if sorted(ignored) != sorted(set(ignored)):
        errors.append(".cargo/audit.toml ignore list has duplicate entries")
    if register_valid and set(registered) != set(ignored):
        errors.append(
            "accepted-advisory register and .cargo/audit.toml ignore list must match; "
            f"register={sorted(set(registered))} ignore={sorted(set(ignored))}"
        )


def check_workflow_commands(root: Path, errors: list[str]) -> None:
    """Pin every workflow file, not the lines this script knows to read.

    Checking selected lines only binds what was thought of: the two `run`
    values can stay verbatim while an `if: ${{ false }}` sibling stops either
    step from executing. Python has no standard YAML parser, and adding one
    before Cargo runs would place another interpreter in the boundary, so the
    files are pinned by digest instead.
    """
    directory = root / ".github/workflows"
    try:
        entries = sorted(entry.name for entry in directory.iterdir())
    except OSError as error:
        errors.append(f".github/workflows must be readable: {error}")
        return

    expected = sorted(REVIEWED_WORKFLOW_SHA256)
    if entries != expected:
        errors.append(
            f".github/workflows entries must be exactly {expected}; found {entries}"
        )

    for name, expected_digest in REVIEWED_WORKFLOW_SHA256.items():
        path = directory / name
        try:
            content = path.read_bytes()
        except OSError as error:
            errors.append(f".github/workflows/{name} must be readable: {error}")
            continue
        digest = hashlib.sha256(content).hexdigest()
        if digest != expected_digest:
            errors.append(
                f".github/workflows/{name} must match the reviewed workflow digest "
                f"{expected_digest}; found {digest}"
            )

def check_release_manifest_policy(root: Path, errors: list[str]) -> None:
    manifest = load_toml(root / "Cargo.toml", "Cargo.toml", errors)
    if manifest is None:
        return
    package = manifest.get("package")
    if not isinstance(package, dict) or package.get("publish") is not False:
        errors.append("Cargo.toml package.publish must be boolean false")

    dependencies = manifest.get("dependencies")
    ethers = dependencies.get("ethers") if isinstance(dependencies, dict) else None
    if not isinstance(ethers, dict):
        errors.append("Cargo.toml dependencies.ethers must be a table")
        return
    if ethers.get("default-features") is not False:
        errors.append("Cargo.toml dependencies.ethers.default-features must be boolean false")
    features = ethers.get("features")
    if not isinstance(features, list) or "openssl" not in features:
        errors.append('Cargo.toml dependencies.ethers.features must include "openssl"')


def check_license_files(root: Path, errors: list[str]) -> None:
    for relative in ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE"):
        path = root / relative
        try:
            contents = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{relative} must be readable: {error}")
            continue
        if not contents.strip():
            errors.append(f"{relative} must not be empty")


def main() -> int:
    root = Path.cwd()
    errors: list[str] = []
    check_committed_tree(root, errors)
    check_no_symlinks(root, errors)
    check_no_build_scripts(root, errors)
    check_no_local_path_dependencies(root, errors)
    check_toolchain(root, errors)
    check_cargo_directory(root, errors)
    check_scripts_directory(root, errors)
    check_manifest_and_test(root, errors)
    check_advisory_register(root, errors)
    check_workflow_commands(root, errors)
    check_release_manifest_policy(root, errors)
    check_license_files(root, errors)

    if errors:
        for error in errors:
            print(f"preflight build integrity: {error}", file=sys.stderr)
        return 1
    print("preflight build integrity: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
