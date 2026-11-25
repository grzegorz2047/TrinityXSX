# SPDX-License-Identifier: MIT
"""Convert a freshly initialized host Wine64 prefix into a clean PS5 console prefix seed."""

import argparse
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
from zipfile import ZipFile

from standalone.stage import verify_prefix_seed


CPU_KEY = b"[Software\\\\Microsoft\\\\Wow64\\\\x86]"
PC_CPU = b'@="wow64cpu.dll"'
CONSOLE_CPU = b'@="wowprospero.dll"'
CONSOLE_USER = "prospero"
CONSOLE_USER_DIRS = (
    "AppData/Local/Temp",
    "AppData/LocalLow",
    "AppData/Roaming",
    "Desktop",
    "Documents",
    "Saved Games",
)


def rewrite_system_reg(data: bytes) -> bytes:
    start = data.find(CPU_KEY)
    if start < 0:
        return data
    end = data.find(b"\n[", start + 1)
    end = len(data) if end < 0 else end
    return data[:start] + data[start:end].replace(PC_CPU, CONSOLE_CPU) + data[end:]


def sanitize_symlinks(prefix: Path) -> None:
    """Replace symlinks inside the prefix with real files or empty directories."""
    root = prefix.resolve()
    for directory, subdirs, files in os.walk(root, topdown=True, followlinks=False):
        current = Path(directory)
        # Never walk into dosdevices — it is replaced by .pw-symlinks.
        if current == root and "dosdevices" in subdirs:
            subdirs.remove("dosdevices")
        for name in list(subdirs):
            entry = current / name
            if entry.is_symlink():
                subdirs.remove(name)
                entry.unlink()
                entry.mkdir(exist_ok=True)
        for name in files:
            entry = current / name
            if entry.is_symlink():
                target = entry.resolve(strict=False)
                entry.unlink()
                if target.is_file():
                    shutil.copy2(target, entry)
                elif target.is_dir():
                    entry.mkdir(exist_ok=True)


def overlay_prospero_pe_modules(prefix: Path, release_zip: Path) -> None:
    """Match prefix system32/syswow64 PE modules to prospero-win's bundled Wine 11.17 runtime."""
    sys32 = prefix / "drive_c/windows/system32"
    syswow64 = prefix / "drive_c/windows/syswow64"
    sys32.mkdir(parents=True, exist_ok=True)
    with ZipFile(release_zip) as archive:
        wow_member = "PPSA99995/win/wine/lib/wine/x86_64-windows/wowprospero.dll"
        with archive.open(wow_member) as src, (sys32 / "wowprospero.dll").open("wb") as dst:
            shutil.copyfileobj(src, dst)

        for entry in archive.infolist():
            if entry.is_dir():
                continue
            parts = PurePosixPath(entry.filename).parts
            if len(parts) == 7 and parts[:5] == ("PPSA99995", "win", "wine", "lib", "wine"):
                arch, filename = parts[5], parts[6]
                if arch == "x86_64-windows":
                    target = sys32 / filename
                elif arch == "i386-windows" and syswow64.is_dir():
                    target = syswow64 / filename
                else:
                    continue
                if target.exists():
                    with archive.open(entry) as src, target.open("wb") as dst:
                        shutil.copyfileobj(src, dst)


def build_prefix_seed(
    release_zip: Path,
    output: Path,
    *,
    wine_bin: str = "wine",
    wineserver_bin: str = "wineserver",
) -> None:
    destination = output.resolve()
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(f"Prefix seed already exists: {destination}")
    destination.mkdir(parents=True)

    env = os.environ.copy()
    env.update(
        WINEPREFIX=str(destination),
        WINEARCH="win64",
        USER=CONSOLE_USER,
        LOGNAME=CONSOLE_USER,
        WINEDLLOVERRIDES="mscoree,mshtml=",
        WINEDEBUG="-all",
        DISPLAY="",
    )
    subprocess.run([wine_bin, "wineboot", "--init"], env=env, check=True)
    if shutil.which(wineserver_bin):
        subprocess.run([wineserver_bin, "-w"], env=env, check=True)

    # Strip host-only state and symlinks that PS5 UFS2 / safe_tree rejects.
    dosdevices = destination / "dosdevices"
    if dosdevices.exists():
        shutil.rmtree(dosdevices)
    dosdevices.mkdir()
    (dosdevices / ".pw-symlinks").write_text("c:\t../drive_c\nz:\t/\n", encoding="utf-8", newline="\n")

    sanitize_symlinks(destination)

    # Ensure C:\users\prospero exists even if host wineboot used the host UID username.
    user_root = destination / "drive_c/users" / CONSOLE_USER
    for rel in CONSOLE_USER_DIRS:
        (user_root / rel).mkdir(parents=True, exist_ok=True)

    system_reg = destination / "system.reg"
    system_reg.write_bytes(rewrite_system_reg(system_reg.read_bytes()))

    overlay_prospero_pe_modules(destination, release_zip.resolve())

    # Remove any transient .wineserver socket dirs or lock files if created.
    for extra in (".wineserver", "drive_c/XCL"):
        path = destination / extra
        if path.exists():
            shutil.rmtree(path)

    verify_prefix_seed(destination)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-zip", type=Path, required=True, help="Verified prospero-win-v0.3.0.zip")
    parser.add_argument("--output", type=Path, required=True, help="Output clean prefix seed directory")
    parser.add_argument("--wine", default="wine", help="Host wine binary used for wineboot --init")
    parser.add_argument("--wineserver", default="wineserver", help="Host wineserver binary")
    args = parser.parse_args()
    try:
        build_prefix_seed(
            args.release_zip, args.output, wine_bin=args.wine, wineserver_bin=args.wineserver
        )
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"build_prefix_seed: {error}\n")
    print(f"Built verified console prefix seed: {args.output.resolve()}")


if __name__ == "__main__":
    main()
