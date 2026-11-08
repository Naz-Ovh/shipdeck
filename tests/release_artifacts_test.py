"""Exercise the release gate with valid, missing, and corrupt archives."""

import hashlib
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "agent_artifacts", Path(__file__).resolve().parents[1] / "scripts/check-agent-artifacts.py"
)
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


class AgentArtifactsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.target = GATE.TARGETS[0]

    def archive(self, *, target=None, machine=62, mode=0o755, member=None):
        target = target or self.target
        name = f"shipdeck-agent-{target}.tar.gz"
        path = self.folder / name
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        header[18:20] = machine.to_bytes(2, "little")
        with tarfile.open(path, "w:gz") as packed:
            info = tarfile.TarInfo(member or f"shipdeck-agent-{target}/shipdeck-agent")
            info.size = len(header)
            info.mode = mode
            packed.addfile(info, io.BytesIO(header))
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.folder / f"{name}.sha256").write_text(f"{digest}  {name}\n")
        return path

    def test_all_four_targets(self):
        for target in GATE.TARGETS:
            with self.subTest(target=target):
                self.archive(target=target, machine=62 if target.startswith("x86_64") else 183)
                GATE.check_archive(self.folder, target)

    def test_missing_archive(self):
        with self.assertRaises(FileNotFoundError):
            GATE.check_archive(self.folder, self.target)

    def test_changed_archive(self):
        path = self.archive()
        path.write_bytes(path.read_bytes() + b"corrupt")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            GATE.check_archive(self.folder, self.target)

    def test_wrong_cpu(self):
        self.archive(machine=183)
        with self.assertRaisesRegex(ValueError, "ELF"):
            GATE.check_archive(self.folder, self.target)

    def test_non_executable(self):
        self.archive(mode=0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            GATE.check_archive(self.folder, self.target)

    def test_escaping_path(self):
        self.archive(member="../shipdeck-agent")
        with self.assertRaisesRegex(ValueError, "unsafe"):
            GATE.check_archive(self.folder, self.target)


if __name__ == "__main__":
    unittest.main()
