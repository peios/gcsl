"""Check the assembled GCSL package boundaries and installed language tools."""

import json
import tomllib
import unittest
from pathlib import Path
from typing import Any, ClassVar

EXPECTED_PACKAGES = {"gcsl", "gcsl-debuginfo", "gcsl-debugsource", "gcsl-source"}
BINARIES = {"usr/bin/gcsl"}


class FamilyTests(unittest.TestCase):
    """Inspect produced manifests and provenance instead of recipe text alone."""

    root: ClassVar[Path]
    index: ClassVar[dict[str, Any]]
    packages: ClassVar[dict[str, Any]]
    manifests: ClassVar[dict[str, Any]]

    @classmethod
    def setUpClass(cls) -> None:
        """Read the declared verified store and canonical file maps."""
        cls.root = Path("packages.store")
        cls.index = json.loads((cls.root / "index.json").read_text(encoding="utf-8"))
        cls.packages = {
            package["name"].removeprefix("dev.peios."): package for package in cls.index["packages"]
        }
        cls.manifests = {
            name: tomllib.loads((cls.root / info["manifest"]).read_text(encoding="utf-8"))["files"]
            for name, info in cls.packages.items()
        }

    def test_complete_unsigned_family(self) -> None:
        self.assertEqual(set(self.packages), EXPECTED_PACKAGES)
        self.assertEqual(self.index["package_count"], 4)
        self.assertEqual(len({package["version"] for package in self.packages.values()}), 1)
        self.assertFalse(self.index["release_ready"])
        self.assertEqual(self.index["trust"], "development")
        for name, info in self.packages.items():
            self.assertFalse(info["release_ready"], name)
            expected_arch = "noarch" if name.endswith("-source") else "x86_64"
            self.assertEqual(info["arch"], expected_arch, name)
            if "build" in info:
                self.assertFalse(info["build"]["signed"], name)
                self.assertFalse(info["build"]["release_ready"], name)
                self.assertIn(
                    info["build"]["userspace_abi"], {"peios-native", "debian-development"}
                )

    def test_all_commands_share_one_runtime_package(self) -> None:
        for name, files in self.manifests.items():
            self.assertEqual(BINARIES & files.keys(), BINARIES if name == "gcsl" else set())
            if name == "gcsl-source":
                # Rebuildable source retains authored executable scripts.
                continue
            executables = {path for path, entry in files.items() if entry.get("exec")}
            self.assertEqual(executables, BINARIES if name == "gcsl" else set(), name)

    def test_transport_symlinks_do_not_enter_packages(self) -> None:
        for name, files in self.manifests.items():
            for path, entry in files.items():
                self.assertNotIn("symlink", entry, (name, path))

    def test_runtime_dependencies_and_authority_are_explicit(self) -> None:
        metadata = self.packages["gcsl"]["package_metadata"]
        self.assertEqual(
            metadata["dependencies"],
            {
                "org.gnu.glibc": "*",
                "org.gnu.gcc-libgcc": "*",
            },
        )
        self.assertEqual(metadata["side_effects"], ["man-db"])
        self.assertNotIn("signing", metadata)
        self.assertNotIn("sd_overrides", metadata)
        self.assertFalse(any(path.startswith("var/") for path in self.manifests["gcsl"]))

    def test_manuals_are_complete_and_in_the_correct_sections(self) -> None:
        files = self.manifests["gcsl"]
        for name, section in (("gcsl.1", "1"), ("gcsl.5", "5"), ("gcsl-stdlib.3", "3")):
            path = f"usr/share/man/man{section}/{name}.gz"
            self.assertIn(path, files)
            self.assertEqual(
                [n for n, entries in self.manifests.items() if path in entries], ["gcsl"]
            )

    def test_debug_packages_match_the_combined_runtime(self) -> None:
        for name in ("gcsl-debuginfo", "gcsl-debugsource"):
            self.assertEqual(
                self.packages[name]["package_metadata"]["dependencies"],
                {
                    "dev.peios.gcsl": f"= {self.packages['gcsl']['version']}",
                },
            )
        symbols = [path for path in self.manifests["gcsl-debuginfo"] if path.endswith(".debug")]
        self.assertEqual(len(symbols), 1)
        for path in symbols:
            self.assertRegex(path, r"^usr/lib/debug/\.build-id/[0-9a-f]{2}/[0-9a-f]+\.debug$")
        self.assertTrue(
            any(
                path.startswith("usr/src/debug/dev.peios.gcsl/")
                for path in self.manifests["gcsl-debugsource"]
            )
        )

    def test_debug_sources_cover_each_crate_and_embedded_standard_module(self) -> None:
        files = self.manifests["gcsl-debugsource"]
        prefix = "usr/src/debug/dev.peios.gcsl/"
        for crate in ("gcsl", "gcsl-disk", "gcsl-lsp", "gcsl-cli"):
            source = "main.rs" if crate == "gcsl-cli" else "lib.rs"
            self.assertIn(prefix + f"crates/{crate}/src/{source}", files)
            self.assertIn(prefix + f"crates/{crate}/Cargo.toml", files)
        for module in ("prelude", "fs", "path"):
            self.assertIn(prefix + f"stdlib/{module}.gcsl", files)

    def test_license_notices_cover_the_complete_binary_family(self) -> None:
        expressions = set()
        for name in ("gcsl", "gcsl-debuginfo", "gcsl-debugsource"):
            prefix = f"usr/share/licenses/dev.peios.{name}/"
            self.assertIn(prefix + "LICENSE", self.manifests[name])
            self.assertIn(prefix + "third-party/crate-licenses.tsv", self.manifests[name])
            expressions.add(self.packages[name]["package_metadata"]["package"]["license"])
        self.assertEqual(len(expressions), 1)

    def test_source_package_is_rebuildable_and_clean(self) -> None:
        files = self.manifests["gcsl-source"]
        for relative in (
            "Cargo.lock",
            "Cargo.toml",
            "MODULE.bazel",
            "MODULE.bazel.lock",
            "BUILD.bazel",
            "bazel",
            "packages/stage.py",
            "packages/validate.py",
            "vendor/ppk/src/lib.rs",
            "crates/gcsl-cli/src/main.rs",
            "crates/gcsl/src/lib.rs",
            "stdlib/prelude.gcsl",
            "docs/man/gcsl.1",
        ):
            self.assertIn("usr/src/gcsl/" + relative, files)
        forbidden = {".git", ".trail-cache", ".ruff_cache", "__pycache__", "target", "bazel-out"}
        for path in files:
            self.assertFalse(forbidden & set(Path(path).parts), path)
            self.assertNotIn("specs", Path(path).parts)


if __name__ == "__main__":
    unittest.main()
