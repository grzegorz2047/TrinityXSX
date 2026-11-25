# SPDX-License-Identifier: MIT
"""Build the MIT-licensed native TrinityXSX PS5 launcher (eboot.bin) on Linux or Windows."""

import argparse
from hashlib import file_digest
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory


NATIVE_DIR = Path(__file__).resolve().parent
APP_STUBS = ("libkernel.so", "libSceLibcInternal.so", "libSceSystemService.so")


def build_launcher(
    sdk: Path,
    llvm_bin: Path,
    native_tool: Path,
    output: Path,
    *,
    with_seed: bool = False,
) -> str:
    destination = output.resolve()
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(f"Output already exists: {destination}")

    include = sdk.resolve() / "target/include"
    libs = sdk.resolve() / "target/lib"
    llvm = llvm_bin.resolve()
    suffix = ".exe" if (llvm / "clang.exe").exists() else ""
    clang = llvm / f"clang{suffix}"
    lld = llvm / f"ld.lld{suffix}"
    tool = native_tool.resolve()

    for required in (include, libs, clang, lld, tool):
        if not required.exists():
            raise FileNotFoundError(f"Missing build input: {required}")

    flags = (
        "-target", "x86_64-sie-ps5",
        "-fvisibility-nodllstorageclass=default",
        "-isystem", str(include),
        "-fno-stack-protector",
        "-fno-plt",
        "-femulated-tls",
        "-O2", "-Wall", "-Wextra", "-Werror",
        "-ffunction-sections", "-fdata-sections",
    )

    destination.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=destination.parent, prefix="trinity-launcher-") as temp:
        scratch = Path(temp)
        crt = scratch / "crt0.o"
        entry = scratch / "launcher.o"
        seed = scratch / "prefix_seed.o"
        pie = scratch / "launcher-pie.elf"
        elf = scratch / "launcher-sce.elf"
        pending = scratch / "eboot.bin"

        objects: list[Path] = [crt, entry]
        defines: list[str] = []
        if with_seed:
            defines.append("-DTRINITYXSX_WITH_SEED")
            subprocess.run(
                [clang, *flags, "-std=c11", "-c", NATIVE_DIR / "prefix_seed.c", "-o", seed],
                check=True,
            )
            objects.append(seed)

        subprocess.run(
            [clang, *flags, "-std=c11", "-c", NATIVE_DIR / "crt0.c", "-o", crt],
            check=True,
        )
        subprocess.run(
            [clang, *flags, "-std=c11", *defines, "-c", NATIVE_DIR / "launcher.c", "-o", entry],
            check=True,
        )

        # Link the application stubs explicitly so overlapping Web/system stub names stay unambiguous.
        stub_dir = scratch / "stubs"
        stub_dir.mkdir()
        stubs: list[Path] = []
        for name in APP_STUBS:
            dst = stub_dir / name
            shutil.copy2(libs / name, dst)
            stubs.append(dst)

        subprocess.run(
            [
                lld, "-m", "elf_x86_64", "-pie",
                "-T", NATIVE_DIR / "launcher.ld",
                "--eh-frame-hdr", "-z", "max-page-size=0x4000",
                "-mllvm", "-emulated-tls", "--hash-style=gnu",
                "-e", "_start", "-o", pie,
                *objects, "--as-needed", *stubs,
            ],
            check=True,
        )
        subprocess.run(
            [
                tool, "link", "--in", pie, "--out", elf,
                "--stub-dir", stub_dir,
                "--module-sdk", "0x02000009",
                "--companion-sdk", "0x08050001",
                "--file-name", "eboot.elf",
            ],
            check=True,
        )
        subprocess.run(
            [tool, "self", "--sign", "--in", elf, "--out", pending, "--magic", "0x1D3D154F"],
            check=True,
        )
        subprocess.run([tool, "self", "--inspect", "--file", pending], check=True)
        with pending.open("rb") as stream:
            digest = file_digest(stream, "sha256").hexdigest()
        pending.rename(destination)
    return digest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--llvm-bin", type=Path, required=True)
    parser.add_argument("--native-tool", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--with-seed", action="store_true")
    args = parser.parse_args()
    try:
        digest = build_launcher(
            args.sdk, args.llvm_bin, args.native_tool, args.output, with_seed=args.with_seed
        )
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"build_launcher: {error}\n")
    print(f"{digest}  {args.output.name}")


if __name__ == "__main__":
    main()
