#!/usr/bin/env python3
"""Collect TrinityXSX logs from a PS5 over FTP into a shareable ZIP for bug reports.

Usage:
    python tools/collect_ps5_logs.py <PS5-IP> [--port 2121] [--out trinityxsx-logs.zip]

Collects only diagnostic logs and configuration files (never game binaries or save files):
- /data/homebrew/PPSA83601/startup.log
- /data/homebrew/PPSA83601/wineserver-startup.log
- /data/homebrew/PPSA83601/game_paths.txt
- /data/homebrew/PPSA83601/logs/next.txt and latest session-*.log files
- /data/homebrew/PPSA83601/prefix/drive_c/XCL/vkd3d-trinityxsx-library.log
- /data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<Title>/{xcl.toml,logs/xcl.log,logs/report.json}
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import ftplib
import io
from pathlib import Path
import zipfile


BASE = "/data/homebrew/PPSA83601"
XCL_TITLES = f"{BASE}/prefix/drive_c/users/prospero/AppData/Local/XCL"
MAX_FILE_BYTES = 16 * 1024 * 1024  # cap individual logs at 16 MiB


def fetch_bytes(ftp: ftplib.FTP, remote_path: str) -> bytes | None:
    buf = io.BytesIO()
    try:
        ftp.retrbinary(f"RETR {remote_path}", buf.write)
    except ftplib.error_perm:
        return None
    data = buf.getvalue()
    if len(data) > MAX_FILE_BYTES:
        head = data[: 2 * 1024 * 1024]
        tail = data[-10 * 1024 * 1024 :]
        return head + b"\n... [TRUNCATED BY collect_ps5_logs.py] ...\n" + tail
    return data


def list_entries(ftp: ftplib.FTP, remote_dir: str) -> list[tuple[bool, str]]:
    lines: list[str] = []
    try:
        ftp.dir(remote_dir, lines.append)
    except ftplib.error_perm:
        return []
    out: list[tuple[bool, str]] = []
    for line in lines:
        parts = line.split(None, 8)
        if len(parts) < 9:
            continue
        name = parts[8].strip()
        if name in (".", ".."):
            continue
        out.append((line.startswith("d"), name))
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", help="PS5 IP address on your local network")
    parser.add_argument("--port", type=int, default=2121, help="PS5 FTP port (default: 2121)")
    parser.add_argument("--out", type=Path, help="Output ZIP file path")
    args = parser.parse_args()

    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%SZ")
    out_zip = args.out or Path(f"trinityxsx-logs-{stamp}.zip")

    ftp = ftplib.FTP()
    print(f"Connecting to PS5 FTP at {args.host}:{args.port} ...")
    ftp.connect(args.host, args.port, timeout=15)
    ftp.login()
    ftp.set_pasv(True)

    collected: list[tuple[str, bytes]] = []

    def try_add(remote_path: str, archive_name: str) -> None:
        data = fetch_bytes(ftp, remote_path)
        if data is not None:
            collected.append((archive_name, data))
            print(f"  + {archive_name} ({len(data):,} bytes)")

    try_add(f"{BASE}/startup.log", "startup.log")
    try_add(f"{BASE}/wineserver-startup.log", "wineserver-startup.log")
    try_add(f"{BASE}/game_paths.txt", "game_paths.txt")
    try_add(f"{BASE}/TRINITYXSX-BUILD.json", "TRINITYXSX-BUILD.json")
    try_add(f"{BASE}/logs/next.txt", "logs/next.txt")
    try_add(f"{BASE}/prefix/drive_c/XCL/vkd3d-trinityxsx-library.log", "vkd3d-trinityxsx-library.log")

    for is_dir, name in sorted(list_entries(ftp, f"{BASE}/logs")):
        if not is_dir and name.startswith("session-") and name.endswith(".log"):
            try_add(f"{BASE}/logs/{name}", f"logs/{name}")

    for is_dir, title in sorted(list_entries(ftp, XCL_TITLES)):
        if not is_dir or title.lower() == "dxil":
            continue
        try_add(f"{XCL_TITLES}/{title}/xcl.toml", f"titles/{title}/xcl.toml")
        try_add(f"{XCL_TITLES}/{title}/logs/xcl.log", f"titles/{title}/xcl.log")
        try_add(f"{XCL_TITLES}/{title}/logs/report.json", f"titles/{title}/report.json")

    ftp.quit()

    if not collected:
        print("No TrinityXSX logs found under /data/homebrew/PPSA83601.")
        return 1

    with zipfile.ZipFile(out_zip, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for arcname, data in collected:
            zf.writestr(arcname, data)

    print(f"\nSaved {len(collected)} log/config file(s) to {out_zip.resolve()}")
    print("Attach this ZIP to your GitHub issue (it contains no game files or saves).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
