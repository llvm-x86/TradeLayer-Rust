#!/usr/bin/env python3
"""Fail-closed, dependency-free audit of the consensus crate's Cargo inputs.

Current admission policy: exactly ONE first-party Cargo package and ZERO
external crates of any kind. No build.rs, proc-macros, alternate registries,
Git/path packages, target-specific dependencies, patches, or vendored code.

Cargo's --frozen independently validates lockfile resolution and prevents
network access. This script also checks manifest omissions/extra packages
that a matching lockfile alone would not necessarily make visible.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib

EXPECTED_TOOLCHAIN = "1.90.0"
EXPECTED_PACKAGE = "tradelayer-core"
EXPECTED_VERSION = "0.1.0"


class SupplyChainError(ValueError):
    pass


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise SupplyChainError(reason)


def read_toml(path: Path) -> dict:
    require(path.is_file(), f"required policy input missing: {path}")
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
        raise SupplyChainError(f"cannot parse {path}: {exc}") from exc
    require(isinstance(data, dict), f"not a TOML table: {path}")
    return data


def validate_files(root: Path) -> None:
    policy = read_toml(root / "security" / "cargo-dependency-policy.toml")
    require(
        policy
        == {
            "schema": 1,
            "policy": "deny-all-external-rust-crates",
            "package": EXPECTED_PACKAGE,
            "package_version": EXPECTED_VERSION,
            "toolchain": EXPECTED_TOOLCHAIN,
            "require_offline": True,
            "deny_build_scripts": True,
            "deny_git_path_registry_dependencies": True,
            "approved_external_crates": [],
        },
        "supply-chain policy changed: requires explicit policy/security review",
    )

    config = read_toml(root / ".cargo" / "config.toml")
    require(config == {"net": {"offline": True}}, "Cargo must run offline with no other local config settings")
    extra_config_files = sorted(
        p.relative_to(root).as_posix()
        for p in (root / ".cargo").rglob("*")
        if p.is_file() and p.name != "config.toml"
    )
    require(not extra_config_files, f"unexpected .cargo configuration/credentials: {extra_config_files}")

    toolchain = read_toml(root / "rust-toolchain.toml")
    require(
        toolchain == {"toolchain": {
            "channel": EXPECTED_TOOLCHAIN,
            "profile": "minimal",
            "components": ["rustfmt", "clippy"],
        }},
        "Rust toolchain profile/version/components differ from the approved pin",
    )

    root_manifest = root / "Cargo.toml"
    manifests = sorted(
        p.relative_to(root).as_posix()
        for p in root.rglob("Cargo.toml")
        if ".git" not in p.relative_to(root).parts
        and "target" not in p.relative_to(root).parts
    )
    require(manifests == ["Cargo.toml"], f"unexpected additional Cargo package/workspace: {manifests}")
    build_scripts = sorted(
        p.relative_to(root).as_posix()
        for p in root.rglob("build.rs")
        if ".git" not in p.relative_to(root).parts
        and "target" not in p.relative_to(root).parts
    )
    require(not build_scripts, f"unreviewed Rust build scripts found: {build_scripts}")
    require(not (root / "vendor").exists(), "third-party vendoring is forbidden in the zero-crate baseline")

    manifest = read_toml(root_manifest)
    require(
        set(manifest) == {"package", "lib", "dependencies"},
        f"unexpected Cargo manifest sections: {sorted(manifest)}",
    )
    require(
        manifest["package"] == {
            "name": EXPECTED_PACKAGE,
            "version": EXPECTED_VERSION,
            "edition": "2021",
            "rust-version": "1.90",
            "description": "A deterministic Rust core for the TradeLayer protocol",
            "publish": False,
        },
        "package metadata/version/edition/MSRV/build policy differs from baseline",
    )
    require(
        manifest["lib"] == {"name": "tradelayer_core", "path": "src/lib.rs"},
        "library entry point differs from baseline",
    )
    require(manifest["dependencies"] == {}, "external Cargo dependencies forbidden in the consensus crate")

    lock = read_toml(root / "Cargo.lock")
    require(set(lock) == {"version", "package"}, "Cargo.lock acquired unauthorized metadata/entries")
    require(lock["version"] == 4, "Cargo.lock version changed")
    require(
        lock["package"] == [{"name": EXPECTED_PACKAGE, "version": EXPECTED_VERSION}],
        "Cargo.lock dependency graph no longer contains exactly the root crate",
    )


def verify_runtime(root: Path) -> None:
    commands = [
        (["rustc", "--version", "--verbose"], "release: " + EXPECTED_TOOLCHAIN),
        (["cargo", "--version"], "cargo " + EXPECTED_TOOLCHAIN + " "),
    ]
    for command, expected in commands:
        try:
            result = subprocess.run(
                command, cwd=root, capture_output=True, text=True,
                check=False, timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise SupplyChainError(f"cannot inspect toolchain {command}: {exc}") from exc
        require(result.returncode == 0 and expected in result.stdout,
                f"toolchain version mismatch for {command}: {result.stdout.strip()} {result.stderr.strip()}")

    try:
        result = subprocess.run(
            ["cargo", "metadata", "--frozen", "--format-version", "1"],
            cwd=root, capture_output=True, text=True, check=False, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise SupplyChainError(f"cargo metadata failed to launch: {exc}") from exc
    require(result.returncode == 0, f"frozen Cargo metadata failed: {result.stderr.strip()}")
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SupplyChainError(f"non-JSON cargo metadata response: {exc}") from exc
    packages = metadata.get("packages", [])
    require(len(packages) == 1, f"Cargo resolved {len(packages)} packages rather than one")
    package = packages[0]
    require(
        package.get("name") == EXPECTED_PACKAGE
        and package.get("version") == EXPECTED_VERSION
        and package.get("source") is None
        and package.get("dependencies") == [],
        "Cargo metadata contains a third-party or alternate package/dependency",
    )
    targets = package.get("targets", [])
    require(
        all("custom-build" not in t.get("kind", []) and "proc-macro" not in t.get("kind", []) for t in targets),
        "cargo metadata includes a build script or procedural macro",
    )
    resolved = metadata.get("resolve") or {}
    require(
        len(resolved.get("nodes", [])) == 1
        and resolved["nodes"][0].get("deps") == [],
        "Cargo resolve graph has unauthorized edges",
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument(
        "--skip-cargo", action="store_true",
        help="For isolated unit fixtures only; CI calls the normal runtime verifier.",
    )
    args = parser.parse_args(argv)
    root = args.root.resolve()
    try:
        validate_files(root)
        if not args.skip_cargo:
            verify_runtime(root)
    except SupplyChainError as exc:
        print(f"DENY: {exc}", file=sys.stderr)
        return 1
    print("PASS: one first-party Rust package, zero third-party crates; offline, pinned and frozen")
    return 0


if __name__ == "__main__":
    sys.exit(main())
