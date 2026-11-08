#!/usr/bin/env python3
"""Refuse a release without all four verified Linux agent archives."""

import hashlib
from pathlib import Path
import sys
import tarfile

TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
)


def check_archive(folder: Path, target: str) -> None:
    """Check the download checksum, executable member, and ELF CPU."""
    name = f"shipdeck-agent-{target}.tar.gz"
    archive = folder / name
    checksum = folder / f"{name}.sha256"
    expected = checksum.read_text().split()[0]
    with archive.open("rb") as stream:
        actual = hashlib.file_digest(stream, "sha256").hexdigest()
    if expected != actual:
        raise ValueError(f"{name}: SHA-256 mismatch")
    with tarfile.open(archive, "r:gz") as packed:
        members = [item for item in packed.getmembers()
                   if Path(item.name).name == "shipdeck-agent"]
        if len(members) != 1:
            raise ValueError(f"{name}: expected exactly one agent")
        member = members[0]
        if not member.isfile() or not member.mode & 0o111:
            raise ValueError(f"{name}: agent must be a regular executable")
        path = Path(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise ValueError(f"{name}: unsafe agent path")
        stream = packed.extractfile(member)
        if stream is None:
            raise ValueError(f"{name}: agent cannot be read")
        with stream:
            header = stream.read(20)
        machine = 62 if target.startswith("x86_64-") else 183
        if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
                or int.from_bytes(header[18:20], "little") != machine):
            raise ValueError(f"{name}: expected a matching 64-bit Linux ELF")


def main() -> None:
    """Validate the exact archive names used by the engine downloader."""
    folder = Path(sys.argv[1])
    for target in TARGETS:
        check_archive(folder, target)
        print(f"verified shipdeck-agent-{target}.tar.gz")


if __name__ == "__main__":
    main()
