# SPDX-License-Identifier: MIT
"""Install a new TrinityXSX library entry in the existing prospero-win catalog."""

import argparse
from datetime import datetime
from ftplib import FTP, all_errors, error_perm
from hashlib import sha256
from io import BytesIO
from pathlib import Path
from uuid import uuid4

from .stage import LIBRARY_EXE, LIBRARY_PROFILE, ROOT, read_library_executable

PROFILES = "/data/prospero-win/profiles"
EXE = f"/data/prospero-win/prefix/drive_c/XCL/{LIBRARY_EXE}"


def read(ftp: FTP, path: str) -> bytes:
    with BytesIO() as stream:
        ftp.retrbinary(f"RETR {path}", stream.write)
        return stream.getvalue()


def missing(ftp: FTP, path: str) -> bool:
    ftp.voidcmd("TYPE I")
    try:
        ftp.size(path)
    except error_perm as error:
        if str(error).startswith("550 "):
            return True
        raise
    return False


def verified_upload(ftp: FTP, path: str, data: bytes) -> None:
    ftp.storbinary(f"STOR {path}", BytesIO(data), blocksize=1 << 20)
    retrieved = sha256()
    ftp.retrbinary(f"RETR {path}", retrieved.update, blocksize=1 << 20)
    if retrieved.digest() != sha256(data).digest():
        raise ValueError(f"Checksum mismatch for {path}")


def deploy(host: str, port: int, binary: Path, replace: bool) -> None:
    code = read_library_executable(binary)
    profile = (ROOT / "configs" / LIBRARY_PROFILE).read_bytes()
    stamp = uuid4().hex[:8]
    index = f"{PROFILES}/profiles.lst"
    profile_dest = f"{PROFILES}/{LIBRARY_PROFILE}"
    pending_index = f"{index}.pending-{stamp}"
    previous_index = f"{index}.before-library-{stamp}"

    with FTP() as ftp:
        ftp.connect(host, port, timeout=30)
        ftp.login()
        before = read(ftp, index)
        if replace:
            if LIBRARY_PROFILE.encode() not in before.splitlines() or read(ftp, profile_dest) != profile:
                raise ValueError("The installed TrinityXSX profile differs from this version")
            backup = ROOT / "ps5-logs/minecraft" / f"library-update-{datetime.now():%Y%m%d-%H%M%S}-{stamp}"
            backup.mkdir(parents=True, exist_ok=False)
            (backup / "xcl-library.previous.exe").write_bytes(read(ftp, EXE))
            pending = f"{EXE}.pending-{stamp}"
            previous = f"{EXE}.previous-{stamp}"
            verified_upload(ftp, pending, code)
            ftp.rename(EXE, previous)
            try:
                ftp.rename(pending, EXE)
            except all_errors:
                ftp.rename(previous, EXE)
                raise
            (backup / "deployment.txt").write_text(f"sha256={sha256(code).hexdigest()}\nprevious_binary={previous}\n", encoding="utf-8")
            print(f"Updated TrinityXSX library with SHA-256 {sha256(code).hexdigest()}")
            print(f"Backup: {backup}")
            return
        if LIBRARY_PROFILE.encode() in before.splitlines() or any(not missing(ftp, path) for path in (EXE, profile_dest)):
            raise ValueError("TrinityXSX library is already present. Use a verified update of the existing files.")
        backup = ROOT / "ps5-logs/minecraft" / f"library-install-{datetime.now():%Y%m%d-%H%M%S}-{stamp}"
        backup.mkdir(parents=True, exist_ok=False)
        (backup / "profiles.lst").write_bytes(before)
        newline = b"\r\n" if b"\r\n" in before else b"\n"
        after = before + (b"" if not before or before.endswith(b"\n") else newline) + LIBRARY_PROFILE.encode() + newline

        for dest, data in ((EXE, code), (profile_dest, profile)):
            pending = f"{dest}.pending-{stamp}"
            verified_upload(ftp, pending, data)
            if not missing(ftp, dest):
                raise ValueError(f"Remote file appeared during transfer: {dest}")
            ftp.rename(pending, dest)
        verified_upload(ftp, pending_index, after)
        if read(ftp, index) != before:
            raise ValueError("The prospero-win catalog changed during transfer")
        ftp.rename(index, previous_index)
        try:
            ftp.rename(pending_index, index)
        except all_errors:
            ftp.rename(previous_index, index)
            raise
        if read(ftp, index) != after:
            raise ValueError("The published catalog differs from the verified index")
    (backup / "deployment.txt").write_text(f"sha256={sha256(code).hexdigest()}\nprofile={profile_dest}\nold_index={previous_index}\n", encoding="utf-8")
    print(f"Installed TrinityXSX library with SHA-256 {sha256(code).hexdigest()}")
    print(f"Backup: {backup}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host")
    parser.add_argument("binary", type=Path)
    parser.add_argument("--port", type=int, default=2121)
    parser.add_argument("--replace", action="store_true", help="Replace only the library executable and preserve the previous binary")
    args = parser.parse_args()
    try:
        deploy(args.host, args.port, args.binary, args.replace)
    except (*all_errors, ValueError) as error:
        parser.exit(1, f"deploy: {error}\n")


if __name__ == "__main__":
    main()
