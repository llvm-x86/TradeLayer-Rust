"""Mutation tests for the deny-by-default Rust dependency policy.

Stdlib-only: uses temporary fixture trees and never downloads or builds crates.
"""

import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check_rust_supply_chain.py"
spec = importlib.util.spec_from_file_location("check_rust_supply_chain", SCRIPT)
assert spec is not None and spec.loader is not None
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class SupplyChainGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tradelayer-cargo-gate-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for relative in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "security/cargo-dependency-policy.toml",
        ]:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)

    def append(self, filename: str, text: str):
        path = self.root / filename
        with path.open("a", encoding="utf-8") as stream:
            stream.write(text)

    def overwrite(self, filename: str, before: str, after: str):
        path = self.root / filename
        body = path.read_text(encoding="utf-8")
        self.assertIn(before, body)
        path.write_text(body.replace(before, after), encoding="utf-8")

    def assert_denied(self):
        with self.assertRaises(gate.SupplyChainError):
            gate.validate_files(self.root)

    def test_pristine_tree_allowed_without_cargo_network(self):
        gate.validate_files(self.root)

    def test_manifest_dependency_poisoning(self):
        for change in [
            "serde = \"=1.0.0\"\n",
            "serde = { version = \"=1.0.0\", default-features = false }\n",
            "serde = { git = \"https://example.com/serde\", rev = \"deadbeef\" }\n",
            "serde = { path = \"../rogue\" }\n",
        ]:
            with self.subTest(change=change):
                self.append("Cargo.toml", change)
                self.assert_denied()
                self.overwrite("Cargo.toml", change, "")

    def test_hidden_manifest_sections_are_denied(self):
        for change in [
            "\n[build-dependencies]\ncc = \"=1.0.0\"\n",
            "\n[dev-dependencies]\nrand = \"=0.8.0\"\n",
            "\n[target.'cfg(unix)'.dependencies]\nlibc = \"=0.2.0\"\n",
            "\n[patch.crates-io]\nserde = { path = \"./evil\" }\n",
            "\n[workspace]\nmembers = [\"vendor/*\"]\n",
            "\n[features]\ndefault = [\"hidden\"]\n",
            "\n[[bin]]\nname = \"fetched-tool\"\npath = \"src/bin/fetched.rs\"\n",
        ]:
            with self.subTest(change=change):
                self.append("Cargo.toml", change)
                self.assert_denied()
                self.overwrite("Cargo.toml", change, "")

    def test_lockfile_registry_or_checksum_change_denied(self):
        for change in [
            "\n[[package]]\nname = \"malicious-crate\"\nversion = \"1.0.0\"\n"
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            "checksum = \"0\"\n",
            "\n[[package]]\nname = \"tradelayer-core\"\nversion = \"0.1.0\"\n",
            "\n[metadata]\nextra = \"unreviewed\"\n",
        ]:
            with self.subTest(change=change):
                self.append("Cargo.lock", change)
                self.assert_denied()
                self.overwrite("Cargo.lock", change, "")

    def test_build_script_and_rogue_subcrate_denied(self):
        self.append("build.rs", "fn main() {}\n")
        self.assert_denied()
        (self.root / "build.rs").unlink()
        path = self.root / "sub" / "Cargo.toml"
        path.parent.mkdir()
        path.write_text("[package]\nname = 'hidden'\nversion = '1.0.0'\n")
        self.assert_denied()
        path.unlink()

    def test_toolchain_replacement_denied(self):
        self.overwrite("rust-toolchain.toml", "1.90.0", "stable")
        self.assert_denied()

    def test_network_allowing_config_denied(self):
        self.overwrite(".cargo/config.toml", "offline = true", "offline = false")
        self.assert_denied()

    def test_cargo_config_alternate_registry_and_credentials_denied(self):
        self.append(".cargo/config.toml", "\n[source.crates-io]\nreplace-with='remote'\n")
        self.assert_denied()
        self.overwrite(".cargo/config.toml", "\n[source.crates-io]\nreplace-with='remote'\n", "")
        (self.root / ".cargo" / "credentials.toml").write_text("secret = 'fake'\n")
        self.assert_denied()

    def test_policy_admission_change_denied(self):
        self.overwrite(
            "security/cargo-dependency-policy.toml",
            "approved_external_crates = []",
            'approved_external_crates = ["serde"]',
        )
        self.assert_denied()

    def test_root_version_and_publish_policy_denied(self):
        self.overwrite("Cargo.toml", 'publish = false', 'publish = true')
        self.assert_denied()

    def test_vendor_directory_denied_in_zero_crate_mode(self):
        (self.root / "vendor").mkdir()
        self.assert_denied()


if __name__ == "__main__":
    unittest.main()
