# SPDX-License-Identifier: MIT
"""Build a read-only UFS2 image from a packaged TrinityXSX runner."""

import argparse
from hashlib import file_digest
from pathlib import Path
import re
import shutil
import subprocess
from tempfile import TemporaryDirectory
import zipfile

from .stage import (
    LIBRARY_EXE, LIBRARY_PROFILE, RUNNER_EXECUTABLE, TITLE_ID, copy_title, safe_tree,
    verify_launcher, verify_runner,
)


def write_app_zip(title_dir: Path, zip_output: Path) -> str:
    """Package the staged `<TITLEID>/` app directory as a ProsperoStore-compatible `.zip`."""
    if zip_output.name != f"{TITLE_ID}.zip":
        raise ValueError(f"Use a zip output named {TITLE_ID}.zip")
    if zip_output.exists() or zip_output.is_symlink():
        raise FileExistsError(f"Zip already exists: {zip_output}")
    zip_output.parent.mkdir(parents=True, exist_ok=True)
    pending = zip_output.with_suffix(".zip.part")
    try:
        with zipfile.ZipFile(pending, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
            for entry in sorted(title_dir.rglob("*")):
                if entry.is_dir():
                    continue
                relative = entry.relative_to(title_dir).as_posix()
                arcname = f"{TITLE_ID}/{relative}"
                info = zipfile.ZipInfo(arcname)
                info.compress_type = zipfile.ZIP_DEFLATED
                info.create_system = 3  # Unix permissions
                mode = 0o755 if (
                    relative in {"eboot.bin", RUNNER_EXECUTABLE}
                    or entry.suffix.casefold() in {".prx", ".elf"}
                ) else 0o644
                info.external_attr = (0o100000 | mode) << 16
                archive.writestr(info, entry.read_bytes())
        with zipfile.ZipFile(pending, "r") as check:
            names = set(check.namelist())
            for required in (f"{TITLE_ID}/eboot.bin", f"{TITLE_ID}/sce_sys/param.json", f"{TITLE_ID}/sce_sys/icon0.png"):
                if required not in names:
                    raise ValueError(f"Zip is missing required entry: {required}")
        with pending.open("rb") as stream:
            digest = file_digest(stream, "sha256").hexdigest()
        pending.rename(zip_output)
        return digest
    except Exception:
        pending.unlink(missing_ok=True)
        raise


def build_image(
    runner: Path, output: Path, ufs2tool: Path, *, launcher: Path | None = None, data_seed: Path | None = None,
    content_version: str | None = None, zip_output: Path | None = None,
) -> str:
    verify_runner(runner)
    if content_version is not None and (
        launcher is None or not re.fullmatch(r"[0-9]{2}\.[0-9]{3}\.[0-9]{3}", content_version)
    ):
        raise ValueError("A content version requires the launcher and the format NN.NNN.NNN")
    if launcher is not None:
        verify_launcher(runner, launcher)
    if data_seed is not None:
        if launcher is None:
            raise ValueError("An embedded data seed requires the native launcher")
        safe_tree(data_seed)
        if ((data_seed / "profiles/profiles.lst").read_text(encoding="utf-8").splitlines() != [LIBRARY_PROFILE]
                or not (data_seed / "prefix/system.reg").is_file()
                or not (data_seed / "prefix/drive_c/XCL" / LIBRARY_EXE).is_file()):
            raise ValueError("Expected a library data stage prepared with a console prefix seed")
    if output.name != f"{TITLE_ID}.ffpkg" or output.resolve().is_relative_to(runner.resolve()):
        raise ValueError(f"Use an output named {TITLE_ID}.ffpkg outside the runner folder")
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"Image already exists: {output}")
    if not ufs2tool.is_file():
        raise ValueError(f"UFS2Tool not found: {ufs2tool}")
    command = [str(ufs2tool.resolve())]
    if ufs2tool.suffix.casefold() == ".dll":
        command = ["dotnet", "--roll-forward", "Major", *command]

    def run(*args: str) -> None:
        subprocess.run([*command, *args], check=True)

    output.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=output.parent, prefix="trinity-image-") as scratch:
        pending = Path(scratch) / output.name
        title = runner.resolve()
        if launcher is not None:
            title = Path(scratch) / TITLE_ID
            copy_title(runner, title, launcher, content_version=content_version)
        if data_seed is not None:
            for name in ("prefix", "profiles"):
                shutil.copytree(data_seed / name, title / "data-seed" / name)
        if zip_output is not None:
            write_app_zip(title, zip_output)
        # Match the layout used by TrinityX360's source release and its installed image.
        run("newfs", "-O", "2", "-b", "65536", "-f", "8192",
            "-D", str(title), str(pending), TITLE_ID)
        # Windows input folders cannot reliably carry Unix executable mode bits.
        for entry in sorted(title.rglob("*")):
            relative = entry.relative_to(title).as_posix()
            if relative in {"eboot.bin", RUNNER_EXECUTABLE} or entry.suffix.casefold() in {".prx", ".elf"}:
                run("chmod", str(pending), "755", "/" + relative)
        run("fsck_ufs", "-n", str(pending))
        for name in ("eboot.bin", "sce_sys/param.json", "win/wine/lib/wine/x86_64-unix/ntdll.prx"):
            run("stat", str(pending), f"/{name}")
        if launcher is not None:
            run("stat", str(pending), f"/{RUNNER_EXECUTABLE}")
        if data_seed is not None:
            run("stat", str(pending), "/data-seed/profiles/profiles.lst")
            run("stat", str(pending), f"/data-seed/prefix/drive_c/XCL/{LIBRARY_EXE}")
            run("stat", str(pending), "/data-seed/prefix/drive_c/XCL/runtime-stamp.txt")
        with pending.open("rb") as stream:
            digest = file_digest(stream, "sha256").hexdigest()
        if output.exists() or output.is_symlink():
            raise FileExistsError(f"Image already exists: {output}")
        pending.rename(output)
    return digest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=Path)
    parser.add_argument("image", type=Path)
    parser.add_argument("--ufs2tool", type=Path, required=True)
    parser.add_argument("--launcher", type=Path, help="Native TrinityXSX eboot built from standalone/native/launcher.c")
    parser.add_argument("--data-seed", type=Path, help="Library data stage for first-launch initialization")
    parser.add_argument("--content-version", help="Set the content version for the packaged launcher")
    parser.add_argument("--zip-output", type=Path, help="Also emit a ProsperoStore-compatible PPSA83601.zip app archive")
    args = parser.parse_args()
    try:
        digest = build_image(
            args.runner, args.image, args.ufs2tool, launcher=args.launcher,
            data_seed=args.data_seed, content_version=args.content_version,
            zip_output=args.zip_output,
        )
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"image: {error}\n")
    print(f"{digest}  {args.image.name}")


if __name__ == "__main__":
    main()
