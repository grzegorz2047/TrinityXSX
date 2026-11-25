# SPDX-License-Identifier: MIT
"""Upload and verify a private TrinityXSX prototype outside the app library."""

import argparse
from ftplib import FTP, all_errors, error_perm
from hashlib import file_digest, sha256
import json
from pathlib import Path
from uuid import uuid4

from .stage import TITLE_ID, safe_tree


REMOTE_ROOT = "/data/TrinityXSX-staging"
REMOTE_DATA = f"{REMOTE_ROOT}/{TITLE_ID}-data"


def digest_file(path: Path) -> str:
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def folder_exists(ftp: FTP, directory: str) -> bool:
    try:
        ftp.cwd(directory)
    except error_perm as error:
        if str(error).startswith("550 "):
            return False
        raise
    return True


def upload(host: str, port: int, source: Path) -> None:
    safe_tree(source)
    build = json.loads((source / "TRINITYXSX-BUILD.json").read_text(encoding="utf-8"))
    if build.get("title_id") != TITLE_ID or build.get("contains_native_runner_and_prefix_seed") is not False:
        raise ValueError("Expected the prototype data stage from standalone.stage")
    if {entry.name for entry in source.iterdir()} != {"prefix", "profiles", "TRINITYXSX-BUILD.json"}:
        raise ValueError("Unexpected entries in the data stage")
    entries = sorted(source.rglob("*"))
    total = sum(entry.is_file() for entry in entries)

    with FTP() as ftp:
        ftp.connect(host, port, timeout=60)
        ftp.login()
        if not folder_exists(ftp, REMOTE_ROOT):
            ftp.mkd(REMOTE_ROOT)
        if folder_exists(ftp, REMOTE_DATA):
            raise ValueError(f"Remote stage already exists: {REMOTE_DATA}")
        pending = f"{REMOTE_DATA}-pending-{uuid4().hex[:8]}"
        ftp.mkd(pending)
        print(f"Staging to {pending}", flush=True)
        for entry in entries:
            target = f"{pending}/{entry.relative_to(source).as_posix()}"
            if entry.is_dir():
                ftp.mkd(target)
                continue
            expected = digest_file(entry)
            with entry.open("rb") as stream:
                ftp.storbinary(f"STOR {target}", stream, blocksize=1 << 20)
            received = sha256()
            ftp.retrbinary(f"RETR {target}", received.update, blocksize=1 << 20)
            if received.hexdigest() != expected:
                raise ValueError(f"Remote checksum mismatch: {target}")
        if folder_exists(ftp, REMOTE_DATA):
            raise ValueError(f"Remote stage appeared during upload: {REMOTE_DATA}")
        ftp.rename(pending, REMOTE_DATA)
    print(f"Verified {total} files at {REMOTE_DATA}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host")
    parser.add_argument("source", type=Path)
    parser.add_argument("--port", type=int, default=2121)
    args = parser.parse_args()
    try:
        upload(args.host, args.port, args.source)
    except (*all_errors, ValueError) as error:
        parser.exit(1, f"upload: {error}\n")


if __name__ == "__main__":
    main()
