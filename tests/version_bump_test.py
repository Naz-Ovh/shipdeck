"""Exercise the standalone version tool against disposable repository copies."""

import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]


class VersionBumpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build = tempfile.TemporaryDirectory(prefix="shipdeck-version-build-")
        cls.addClassCleanup(cls.build.cleanup)
        cls.binary = Path(cls.build.name) / "bump-version"
        subprocess.run(
            ["rustc", "--edition=2024", "-D", "warnings",
             str(ROOT / "scripts/bump-version.rs"), "-o", str(cls.binary)],
            check=True,
        )

    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory(prefix="shipdeck-version-test-")
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)
        paths = [
            "Cargo.toml", "Cargo.lock", "README.md", "justfile",
            "scripts/bump-version.rs", "docs/visual/prototype/desktop.html",
            "docs/schema/deploy.schema.json", "docs/schema/protocol.schema.json",
            "docs/website/public/deploy.schema.json",
        ]
        paths.extend(str(path.relative_to(ROOT))
                     for path in (ROOT / "examples").glob("*/deploy.yaml"))
        for name in paths:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        self.old = tomllib.loads((self.root / "Cargo.toml").read_text())[
            "workspace"]["package"]["version"]
        self.new = f"{int(self.old.split('.')[0]) + 1}.0.0"

    def snapshot(self):
        return {path.relative_to(self.root): path.read_bytes()
                for path in self.root.rglob("*") if path.is_file()}

    def run_tool(self, *args):
        return subprocess.run([str(self.binary), *args], cwd=self.root,
                              capture_output=True, text=True, check=False)

    def test_bump_preserves_dependency_entries_and_schema_contents(self):
        before = tomllib.loads((self.root / "Cargo.lock").read_text())
        schemas = {path: json.loads(path.read_text())
                   for path in self.root.rglob("*.schema.json")}
        result = self.run_tool(self.new)
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(manifest["workspace"]["package"]["version"], self.new)
        after = tomllib.loads((self.root / "Cargo.lock").read_text())
        for package in before["package"]:
            if package["name"] in (
                "shipdeck", "shipdeck-agent", "shipdeck-engine",
                "shipdeck-protocol", "shipdeck-tui",
            ):
                self.assertNotIn("source", package)
                package["version"] = self.new
        self.assertEqual(before, after)
        for path, original in schemas.items():
            original["$id"] = original["$id"].replace(
                f"/v{self.old}/", f"/v{self.new}/")
            self.assertEqual(original, json.loads(path.read_text()))
        for path in (self.root / "examples").glob("*/deploy.yaml"):
            self.assertIn(f"/v{self.new}/", path.read_text())

    def test_invalid_versions_leave_files_unchanged(self):
        before = self.snapshot()
        for value in (self.old, "0.0.0", "01.2.3", "1.2", "1.2.3-beta", "no"):
            with self.subTest(value=value):
                self.assertNotEqual(self.run_tool(value).returncode, 0)
                self.assertEqual(before, self.snapshot())

    def test_missing_schema_leaves_files_unchanged(self):
        (self.root / "docs/schema/protocol.schema.json").unlink()
        before = self.snapshot()
        self.assertNotEqual(self.run_tool(self.new).returncode, 0)
        self.assertEqual(before, self.snapshot())

    def test_stale_lockfile_leaves_files_unchanged(self):
        path = self.root / "Cargo.lock"
        path.write_text(path.read_text().replace(
            f'name = "shipdeck"\nversion = "{self.old}"',
            'name = "shipdeck"\nversion = "0.0.0"'))
        before = self.snapshot()
        self.assertNotEqual(self.run_tool(self.new).returncode, 0)
        self.assertEqual(before, self.snapshot())

    def test_just_recipe_runs_without_a_cargo_workspace(self):
        result = subprocess.run(["just", "--tempdir", str(self.root),
                                 "version", self.new], cwd=self.root,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"to {self.new}", result.stdout)


if __name__ == "__main__":
    unittest.main()
