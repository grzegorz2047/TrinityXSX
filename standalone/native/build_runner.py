# SPDX-License-Identifier: MIT
"""Build the sandboxed PS5 Wine runner with the local native toolchain."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from standalone.stage import TITLE_ID


SOURCE_REVISION = "404cac6ebbb236d2486d2796fd28b9bd39f2caa6"
WINE_REVISION = "490f6d5dcbb2a5047345b8af88d114bbcaad69a8"
RELEASE_HASH = "b17c5daf19e5053edfbc1ce2588db403ae553592cced39ca92a337bfe1f8eb41"
DATA_ROOT = f"/data/homebrew/{TITLE_ID}"
DOWNLOAD_DATA_MIB = 4096
CONTENT_VERSION = "01.000.004"
REBUILT_SERVER_VERSION = "01.000.006"
SERVER_DIAGNOSTIC_VERSION = "01.000.007"
SERVER_STANDALONE_VERSION = "01.000.012"
SERVER_OWNER_PATCH = "0997-server-ps5-skip-config-dir-ownership.patch"
SERVER_PATCH = "0998-server-ps5-inspect-directories-by-descriptor.patch"
SERVER_DIAGNOSTIC_PATCH = "0999-server-ps5-record-fatal-startup-errors.patch"
SERVER_MODULE = "win/wine/lib/wine/x86_64-unix/wineserver.prx"
NTDLL_MODULE = "win/wine/lib/wine/x86_64-unix/ntdll.prx"
NTDLL_SUBPAGE_PATCH = "0986-ntdll-keep-a-host-page-whole-when-accesses-beside-an-inaccessible-page-run-hot.patch"
SOURCES = (
    "native/wine64_main.c", "native/pw_diagnostics.c", "native/pw_audio_ps5.c",
    "native/pw_data_mount.c", "native/pw_lapy_elevation.c",
    "native/pw_pad_ps5.c", "native/pw_agc_ps5.c", "native/pw_agc_submit_lifecycle.c",
    "native/pw_videoout_ps5.c", "native/pw_wine_display.c", "native/pw_wine_library.c",
    "native/pw_hid_ps5.c", "native/pw_osk_ps5.c", "native/pw_wine_prefix.c",
    "src/pw_result.c", "src/pw_wine_start.c", "src/pw_wine_launch.c", "src/pw_game_profile.c",
    "src/pw_prefix_temp.c", "src/pw_app_profile.c", "src/pw_profile_catalog.c",
    "src/pw_launcher_render.c", "src/pw_present.c", "src/pw_pad.c", "src/pw_game_menu.c",
    "src/pw_osk.c", "src/pw_hid.c", "src/pw_spinner.c", "src/pw_script_input.c",
    "src/pw_tsc_calibrate.c", "wine/ps5/pw_wine_prx.c", "native/ps5log/ps5log.c",
    "native/ps5log/ps5log_ps5_net.c",
)


def rebuilt_server(wine_build: Path) -> tuple[Path, str, bool, bool]:
    report = json.loads((wine_build / "report.json").read_text(encoding="utf-8"))
    patches = report.get("patches", [])
    if report.get("wine_commit") != WINE_REVISION or SERVER_PATCH not in patches:
        raise ValueError("Expected Wine's pinned source revision with the TrinityXSX server patch")
    module = report.get("prx", {}).get("modules", {}).get("wineserver", {})
    checks = ("unresolved", "errors", "title_unbound", "webkit_unbound", "raw_syscalls")
    if not module.get("built") or any(module.get(check) != [] for check in checks):
        raise ValueError("Expected a linked wineserver.prx with a clean title import report")
    path = wine_build / "prx/sce_module/wineserver.prx"
    with path.open("rb") as stream:
        digest = file_digest(stream, "sha256").hexdigest()
    if module.get("sha256") != digest:
        raise ValueError("wineserver.prx does not match the build report SHA-256")
    return path, digest, SERVER_DIAGNOSTIC_PATCH in patches, SERVER_OWNER_PATCH in patches


def rebuilt_ntdll(wine_build: Path) -> tuple[Path, str, bool] | None:
    path = wine_build / "prx/sce_module/ntdll.prx"
    if not path.is_file():
        return None
    report = json.loads((wine_build / "report.json").read_text(encoding="utf-8"))
    patches = report.get("patches", [])
    if report.get("wine_commit") != WINE_REVISION:
        raise ValueError("Expected Wine's pinned source revision for ntdll.prx")
    module = report.get("prx", {}).get("modules", {}).get("ntdll", {})
    checks = ("unresolved", "errors", "title_unbound", "webkit_unbound", "raw_syscalls")
    if not module.get("built") or any(module.get(check) != [] for check in checks):
        raise ValueError("Expected a linked ntdll.prx with a clean title import report")
    with path.open("rb") as stream:
        digest = file_digest(stream, "sha256").hexdigest()
    if module.get("sha256") != digest:
        raise ValueError("ntdll.prx does not match the build report SHA-256")
    return path, digest, NTDLL_SUBPAGE_PATCH in patches


def build(
    source: Path, foundation: Path, sdk: Path, llvm: Path, tool: Path, release: Path, output: Path,
    *, wine_build: Path | None = None, lapy_elf: Path | None = None,
) -> None:
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    with release.open("rb") as stream:
        if file_digest(stream, "sha256").hexdigest() != RELEASE_HASH:
            raise ValueError("Expected the verified prospero-win v0.3.0 release archive")
    revision = subprocess.check_output(["git", "-C", source, "rev-parse", "HEAD"], text=True).strip()
    if revision != SOURCE_REVISION:
        raise ValueError(f"Use the prospero-win source at {SOURCE_REVISION}")
    runner_source = (source / SOURCES[0]).read_text(encoding="utf-8")
    server = rebuilt_server(wine_build) if wine_build else None
    ntdll_mod = rebuilt_ntdll(wine_build) if wine_build else None
    if server and server[2] and "WINE_PS5_SERVER_STARTUP_LOG" not in runner_source:
        raise ValueError("Apply the TrinityXSX server startup diagnostic patch to the runner source")

    suffix = ".exe" if (llvm / "clang.exe").exists() else ""
    clang = llvm / f"clang{suffix}"
    lld = llvm / f"ld.lld{suffix}"
    native = foundation / "tooling/native"
    include = sdk / "target/include"
    libs = sdk / "target/lib"
    output.parent.mkdir(parents=True, exist_ok=True)

    def run(*args):
        subprocess.run(args, check=True)

    with TemporaryDirectory(dir=output.parent, prefix="trinity-runner-") as temporary:
        scratch = Path(temporary)
        runner = scratch / TITLE_ID
        runner.mkdir()
        compile_flags = (
            "-target", "x86_64-sie-ps5", "-fvisibility-nodllstorageclass=default",
            "-isystem", include / "c++/v1", "-isystem", include, "-fno-stack-protector",
            "-fno-plt", "-femulated-tls", "-O2", "-Wall", "-Wextra", "-Werror",
            "-ffunction-sections", "-fdata-sections",
        )
        link_flags = ("-m", "elf_x86_64", "--eh-frame-hdr", "-z", "max-page-size=0x4000",
                      "-mllvm", "-emulated-tls", "--hash-style=gnu")
        imports = []
        for name in ("libSceAgc", "libSceAgcDriver"):
            obj = scratch / f"{name}.o"
            stub = scratch / f"{name}.so"
            run(clang, *compile_flags, "-std=c11", "-fPIC", "-c", source / f"native/stubs/{name}.c", "-o", obj)
            run(lld, *link_flags, "--shared", "-soname", f"{name}.prx", "-o", stub, obj)
            imports.append(stub)

        crt = scratch / "app_crt.o"
        run(clang, *compile_flags, "-std=c++20", "-fno-exceptions", "-fno-rtti", "-c", native / "app_crt.cpp", "-o", crt)
        objects = [crt]
        for name in SOURCES:
            obj = scratch / name.replace("/", "_").replace(".c", ".o")
            extra = ("-include", source / "native/ps5log/ps5log_ps5_net.h") if name.endswith("/ps5log.c") else ()
            run(
                clang, *compile_flags, "-std=c11", *extra,
                "-I", source / "include", "-I", source / "src", "-I", source / "native",
                f'-DPW_TITLE_ID="{TITLE_ID}"', '-DPW_FIXED_PROFILE="trinityxsx-library"',
                f'-DPW_WINE64_ROOT_DATA="{DATA_ROOT}"', f'-DPW_WINE64_ROOT_SANDBOX="{DATA_ROOT}"',
                f'-DPW_BUILD_ID="{SOURCE_REVISION}-trinityxsx"',
                "-c", source / name, "-o", obj,
            )
            objects.append(obj)

        pie = scratch / "runner-pie.elf"
        elf = scratch / "runner-sce.elf"
        run(lld, *link_flags, "-pie", "-T", native / "ps5-pie.ld", "-e", "_start", "-o", pie,
            *objects, "--as-needed", *sorted(libs.glob("*.so")), *imports)
        run(tool, "link", "--in", pie, "--out", elf, "--stub-dir", libs, "--stub", imports[0], "--stub", imports[1],
            "--module-sdk", "0x02000009", "--companion-sdk", "0x08050001", "--file-name", "pw_runner.elf")
        executable = runner / "eboot.bin"
        run(tool, "self", "--sign", "--in", elf, "--out", executable, "--magic", "0x1D3D154F")
        run(tool, "self", "--inspect", "--file", executable)

        with ZipFile(release) as archive:
            for entry in archive.infolist():
                parts = PurePosixPath(entry.filename).parts
                if entry.is_dir() or len(parts) < 2 or parts[0] != "PPSA99995":
                    continue
                if parts[1] not in {"win", "sce_module", "sce_sys", "LICENSE", "LICENSES", "THIRD_PARTY.md", "SOURCES.txt"}:
                    continue
                target = runner.joinpath(*parts[1:])
                if not target.resolve().is_relative_to(runner.resolve()):
                    raise ValueError(f"Unexpected archive member: {entry.filename}")
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as original, target.open("xb") as destination:
                    shutil.copyfileobj(original, destination)
        if server:
            shutil.copyfile(server[0], runner / SERVER_MODULE)
        if ntdll_mod:
            shutil.copyfile(ntdll_mod[0], runner / NTDLL_MODULE)
        if lapy_elf is not None:
            lapy_bytes = lapy_elf.read_bytes()
            if not lapy_bytes.startswith(b"\x7fELF"):
                raise ValueError(f"Expected an ELF file for --lapy-elf: {lapy_elf}")
            (runner / "lapy.elf").write_bytes(lapy_bytes)
        params = runner / "sce_sys/param.json"
        metadata = json.loads(params.read_text(encoding="utf-8"))
        metadata["titleId"] = TITLE_ID
        metadata["conceptId"] = TITLE_ID[4:]
        metadata["contentId"] = metadata["contentId"].replace("PPSA99995", TITLE_ID)
        metadata["downloadDataSize"] = DOWNLOAD_DATA_MIB
        metadata["contentVersion"] = CONTENT_VERSION
        if server:
            if server[3]:
                metadata["contentVersion"] = SERVER_STANDALONE_VERSION
            else:
                metadata["contentVersion"] = SERVER_DIAGNOSTIC_VERSION if server[2] else REBUILT_SERVER_VERSION
        params.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
        with (runner / "SOURCES.txt").open("a", encoding="utf-8") as sources:
            sources.write("\nTrinityXSX native runner: patches/prospero-win/0001-Use-the-title-sandbox.patch\n")
            sources.write(f"Writable library root: {DATA_ROOT}\n")
            if server:
                sources.write(f"TrinityXSX PS5 Wine server: Wine {WINE_REVISION}\n")
                sources.write("Patch: patches/prospero-win/0002-Inspect-server-directories-by-descriptor.patch\n")
                if server[2]:
                    sources.write("Patch: patches/prospero-win/0003-Log-Wine-server-startup-errors.patch\n")
                if server[3]:
                    sources.write("Patch: patches/prospero-win/0004-Skip-config-dir-owner-check-on-PS5.patch\n")
                sources.write(f"wineserver.prx SHA-256: {server[1]}\n")
            if ntdll_mod:
                if ntdll_mod[2]:
                    sources.write("Patch: patches/prospero-win/0011-Backport-ntdll-patch-0986-hot-subpage-access.patch\n")
                sources.write(f"ntdll.prx SHA-256: {ntdll_mod[1]}\n")
        runner.rename(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    names = ("source", "foundation", "sdk", "llvm", "tool", "release", "output")
    for name in names:
        parser.add_argument(name, type=Path)
    parser.add_argument("--wine-build", type=Path, help="PS5 Wine build directory containing report.json and prx/")
    parser.add_argument("--lapy-elf", type=Path, help="MIT lapy.elf one-shot helper built for PPSA83601")
    args = parser.parse_args()
    try:
        build(*(getattr(args, name).resolve() for name in names), wine_build=args.wine_build,
              lapy_elf=args.lapy_elf.resolve() if args.lapy_elf else None)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"build_runner: {error}\n")
    print(f"Built runner: {args.output.resolve()}")


if __name__ == "__main__":
    main()
