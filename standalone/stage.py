# SPDX-License-Identifier: MIT
"""Stage the TrinityXSX library or the original Minecraft test profile."""

import argparse
from hashlib import file_digest, sha256
import json
from pathlib import Path
import shutil
from tempfile import TemporaryDirectory
import tomllib

from .catalog import read_game


ROOT = Path(__file__).resolve().parent.parent
TITLE_ID = "PPSA83601"
GAME_FOLDER = "minecraft-beta"
IDENTITY = "Microsoft.MinecraftBeta"
GAME_ROOT = "prefix/drive_c/XCL/games"
GAME_DESTINATION = f"{GAME_ROOT}/{GAME_FOLDER}"
LIBRARY_PROFILE = "trinityxsx-library.profile"
LIBRARY_EXE = "xcl-library.exe"
RUNNER_EXECUTABLE = "runtime/pw_runner.bin"
DEFAULT_RUNTIME = ROOT / "dist/prospero-win/prefix/drive_c/XCL"
RUNTIME_DLLS = (
    "d3d12.dll", "d3d12core.dll", "dxgi.dll", "d3d11.dll",
    "dxcompiler.dll", "dxil.dll", "opus.dll",
)
RUNTIME_FILES = ("xcl.exe", *RUNTIME_DLLS)
USER_DATA = Path("drive_c/users/prospero/AppData/Local/XCL")


def require_files(root: Path, names: tuple[str, ...]) -> None:
    for name in names:
        file = root / name
        if not file.is_file() or file.stat().st_size == 0:
            raise ValueError(f"Missing or empty input: {file}")


def safe_tree(source: Path) -> None:
    """Pack normal files only. Console prefix links belong in .pw-symlinks tables."""
    if not source.is_dir() or source.is_symlink() or source.is_junction():
        raise ValueError(f"Expected a real directory: {source}")
    for directory, subdirs, files in source.walk(follow_symlinks=False):
        for name in subdirs + files:
            item = directory / name
            if item.is_symlink() or item.is_junction() or not (item.is_dir() or item.is_file()):
                raise ValueError(f"Unsupported package entry: {item}")


def verify_runner(runner: Path) -> None:
    safe_tree(runner)
    require_files(runner, (
        "eboot.bin", "sce_sys/param.json", "sce_sys/icon0.png", "sce_module/libc.prx",
        "win/wine/lib/wine/x86_64-unix/ntdll.prx",
        "win/wine/lib/wine/x86_64-unix/wineps5.prx", "SOURCES.txt",
    ))
    metadata = json.loads((runner / "sce_sys/param.json").read_text(encoding="utf-8-sig"))
    if not isinstance(metadata, dict) or metadata.get("titleId") != TITLE_ID:
        raise ValueError(f"Runner must already be built for {TITLE_ID}")
    writable = {"profiles", "prefix", "prefixes", "games", "logs", "saves", "cache"}
    if any(entry.name.casefold() in writable for entry in runner.iterdir()):
        raise ValueError("Use a clean runner release folder without writable title data")


def verify_launcher(runner: Path, launcher: Path) -> None:
    require_files(launcher.parent, (launcher.name,))
    if launcher.is_symlink():
        raise ValueError("Use a regular file for the TrinityXSX launcher executable")
    code = launcher.read_bytes()
    if not code.startswith((b"\x7fELF", b"\x4f\x15\x3d\x1d")):
        raise ValueError("Use a native ELF or SELF launcher executable")
    if code == (runner / "eboot.bin").read_bytes():
        raise ValueError("Supply the independently built TrinityXSX launcher executable")
    if (runner / RUNNER_EXECUTABLE).exists():
        raise ValueError(f"The supplied runner already contains {RUNNER_EXECUTABLE}")


def copy_title(
    runner: Path, destination: Path, launcher: Path | None, *, content_version: str | None = None,
) -> None:
    shutil.copytree(runner, destination)
    if launcher is None:
        return

    eboot = destination / "eboot.bin"
    secondary = destination / RUNNER_EXECUTABLE
    secondary.parent.mkdir(exist_ok=True)
    eboot.rename(secondary)
    shutil.copy2(launcher, eboot)

    param = destination / "sce_sys/param.json"
    metadata = json.loads(param.read_text(encoding="utf-8-sig"))
    if content_version is not None:
        metadata["contentVersion"] = content_version
    locales = metadata.setdefault("localizedParameters", {})
    if not isinstance(locales, dict):
        raise ValueError("Invalid localizedParameters in runner metadata")
    default_language = locales.setdefault("defaultLanguage", "en-US")
    if not isinstance(default_language, str) or default_language in {"", "defaultLanguage"}:
        raise ValueError("Invalid defaultLanguage in runner metadata")
    locales.setdefault(default_language, {})
    for language, fields in locales.items():
        if language != "defaultLanguage":
            if not isinstance(fields, dict):
                raise ValueError(f"Invalid {language} locale in runner metadata")
            fields["titleName"] = "TrinityXSX"
    param.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    shutil.copy2(ROOT / "standalone/assets/icon0.png", destination / "sce_sys/icon0.png")

    license_path = destination / "licenses/TrinityXSX-native-launcher.txt"
    license_path.parent.mkdir(exist_ok=True)
    with license_path.open("xb") as license_file:
        license_file.write((ROOT / "standalone/LICENSE").read_bytes())


def verify_prefix_seed(seed: Path) -> None:
    safe_tree(seed)
    require_files(seed, ("system.reg", "user.reg", "userdef.reg", "dosdevices/.pw-symlinks"))
    links = (seed / "dosdevices/.pw-symlinks").read_text(encoding="utf-8").splitlines()
    if "c:\t../drive_c" not in links or not (seed / "drive_c/windows").is_dir():
        raise ValueError("Prefix seed is missing the console C: drive mapping or Windows folder")
    if (seed / "drive_c/XCL").exists() or (seed / USER_DATA).exists():
        raise ValueError("Use a clean prefix seed without installed XCL files or saves")


def profile_text(name: str) -> str:
    if any(ord(char) < 32 or ord(char) == 127 for char in name) or len(name.encode("utf-8")) > 96:
        raise ValueError("Game display name does not fit in a prospero-win profile")
    return rf"""; TrinityXSX Minecraft prototype
[application]
id = {GAME_FOLDER}
name = {name}
executable = C:\XCL\xcl.exe
working_directory = C:\XCL
arguments = run --in-process C:\XCL\games\{GAME_FOLDER}
dll_overrides = d3d12,d3d12core,dxgi,d3d11=n
prefix = default
runtime = wine-wow64
architecture = pe64
graphics = dxvk

[display]
desktop = 1920x1080

[input]
mode = xinput

[debug]
winedebug = err+all
env = VKD3D_SHADER_CACHE_PATH=C:/XCL/cache/{GAME_FOLDER}
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d-minecraft.log
env = DXVK_LOG_PATH=C:/XCL
"""


def read_library_executable(binary: Path) -> bytes:
    code = binary.read_bytes()
    if not code.startswith(b"MZ") or b"TrinityXSX: library window opened" not in code:
        raise ValueError("Expected an XCL release executable containing the TrinityXSX library")
    return code


def stage(
    game_folder: Path | None, runtime: Path, output: Path, runner: Path | None, seed: Path | None,
    *, library_exe: Path | None = None, launcher: Path | None = None,
) -> None:
    game = None
    if library_exe is not None and game_folder is None:
        library_code = read_library_executable(library_exe)
        profile_name = LIBRARY_PROFILE
        profile = (ROOT / "configs" / profile_name).read_bytes()
        game_destination = GAME_ROOT
        runtime_files = RUNTIME_DLLS
    elif game_folder is not None and library_exe is None:
        game = read_game(game_folder)
        if (game.identity, game.version, game.title_id, game.executable.casefold()) != (
            IDENTITY, "1.18.3064.0", "639AA8DD", "minecraft.windows.exe",
        ):
            raise ValueError("This prototype only stages the tested Minecraft Preview 1.18.3064.0 build")
        profile_name = f"{GAME_FOLDER}.profile"
        profile = profile_text(game.name).encode("utf-8")
        game_destination = GAME_DESTINATION
        preset = (ROOT / "configs/minecraft.xcl.toml").read_bytes()
        settings = tomllib.loads(preset.decode("utf-8-sig"))
        if (settings["exe"].casefold(), settings["title_id"]) != (game.executable.casefold(), game.title_id):
            raise ValueError("The tested XCL preset does not match the manifest")
        runtime_files = RUNTIME_FILES
    else:
        raise ValueError("Supply a Minecraft game folder or --library-exe")
    require_files(runtime, runtime_files)
    safe_tree(runtime / "licenses")

    if runner is not None and seed is None:
        raise ValueError("Supply --prefix-seed to stage a complete app")
    if seed is not None:
        verify_prefix_seed(seed)
    if launcher is not None and (runner is None or library_exe is None):
        raise ValueError("--launcher requires --runner, --prefix-seed and --library-exe")
    if runner is not None and seed is not None:
        verify_runner(runner)
        if launcher is not None:
            verify_launcher(runner, launcher)
        if output.name != TITLE_ID:
            raise ValueError(f"The complete app directory must be named {TITLE_ID}")
    for source in (game_folder, runtime, runner, seed):
        if source is not None and output.resolve().is_relative_to(source.resolve()):
            raise ValueError(f"Output must be outside the input folder: {source}")
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"Output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=output.parent, prefix="trinity-stage-") as scratch:
        tree = Path(scratch) / TITLE_ID
        if runner is not None and seed is not None:
            copy_title(runner, tree, launcher)
        if seed is not None:
            shutil.copytree(seed, tree / "prefix")
        xcl = tree / "prefix/drive_c/XCL"
        xcl.mkdir(parents=True)
        for name in runtime_files:
            shutil.copy2(runtime / name, xcl / name)
        inno_dll = runtime / "trinity_innoextract.dll"
        if inno_dll.is_file():
            shutil.copy2(inno_dll, xcl / "trinity_innoextract.dll")
        if library_exe is not None:
            (xcl / LIBRARY_EXE).write_bytes(library_code)
        hashes = {}
        for binary in sorted(xcl.iterdir()):
            with binary.open("rb") as stream:
                hashes[binary.name] = file_digest(stream, "sha256").hexdigest()
        stamp = sha256()
        for name in sorted(hashes):
            stamp.update(f"{name}:{hashes[name]}\n".encode("ascii"))
        (xcl / "runtime-stamp.txt").write_text(f"{stamp.hexdigest()}\n", encoding="ascii", newline="\n")
        shutil.copytree(runtime / "licenses", xcl / "licenses")
        (tree / game_destination).mkdir(parents=True)
        if game is not None:
            data = tree / "prefix" / USER_DATA / game.identity
            data.mkdir(parents=True)
            (data / "xcl.toml").write_bytes(preset)
        profiles = tree / "profiles"
        profiles.mkdir()
        (profiles / profile_name).write_bytes(profile)
        (profiles / "profiles.lst").write_text(f"{profile_name}\n", encoding="ascii", newline="\n")
        record = {
            "title_id": TITLE_ID,
            "contains_native_runner_and_prefix_seed": runner is not None,
            "game_install_path": game_destination,
            "xcl_runtime_sha256": hashes,
        }
        if game is not None:
            record.update(game_identity=game.identity, game_version=game.version)
        (tree / "TRINITYXSX-BUILD.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
        if output.exists() or output.is_symlink():
            raise FileExistsError(f"Output already exists: {output}")
        tree.rename(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("game", type=Path, nargs="?", help="Local copy of Minecraft for the original test profile")
    parser.add_argument("output", type=Path, help="New directory for the prototype")
    parser.add_argument("--library-exe", type=Path, help="XCL executable built with the TrinityXSX library patch")
    parser.add_argument("--xcl-runtime", type=Path, default=DEFAULT_RUNTIME)
    parser.add_argument("--runner", type=Path, help="Already packaged prospero-win app for PPSA83601")
    parser.add_argument("--prefix-seed", type=Path, help="Clean console prefix for the supplied Wine build")
    parser.add_argument("--launcher", type=Path, help="Native TrinityXSX eboot built from standalone/native/launcher.c")
    args = parser.parse_args()
    try:
        stage(
            args.game, args.xcl_runtime, args.output, args.runner, args.prefix_seed,
            library_exe=args.library_exe, launcher=args.launcher,
        )
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"stage: {error}\n")
    print(f"Staged: {args.output.resolve()}")
    print(f"Game files go in: {GAME_ROOT if args.library_exe is not None else GAME_DESTINATION}")
    if args.runner is None:
        print("Data stage only. Supply --runner and --prefix-seed for a complete app folder.")


if __name__ == "__main__":
    main()
