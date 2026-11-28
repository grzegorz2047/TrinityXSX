# SPDX-License-Identifier: MIT
"""Decisions and integrity checks for TrinityXSX's draft release workflow
(.github/workflows/release.yml).

    python3 -I tools/release_guard.py version --ref <GITHUB_REF>
    python3 -I tools/release_guard.py content-version --version X.Y.Z
    python3 -I tools/release_guard.py check-repo --version X.Y.Z [--root .]
    python3 -I tools/release_guard.py check-zip-sce-sys --zip PPSA83601.zip --sce-sys sce_sys
    python3 -I tools/release_guard.py sums <asset dir>
    python3 -I tools/release_guard.py verify <asset dir>
    python3 -I tools/release_guard.py decide --tag vX.Y.Z --sha <commit> \\
        --view-exit <code> --view-stdout <file> --view-stderr <file> [--remote-tag <file>]
    python3 -I tools/release_guard.py stale <asset dir> --view-stdout <file>

Adapted from TrinityX360's draft-first release guard:
- Releases are built from a `release/X.Y.Z` branch into a DRAFT GitHub Release,
  never auto-published on tag push. Publishing the draft by hand after console
  testing creates tag `vX.Y.Z` at the exact tested commit.
- A published release, an immutable release, or a tag at another commit is
  refused (exit 2).
- `SHA256SUMS.txt` is generated and strictly re-verified before upload.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import sys
import zipfile
from pathlib import Path

VERSION_RE = re.compile(r"([0-9]+)\.([0-9]+)\.([0-9]+)(?:\.([1-9][0-9]{0,2}))?")
BRANCH_PREFIX = "release/"
SHA_RE = re.compile(r"[0-9a-f]{40}")
ASSET_NAME_RE = re.compile(r"[A-Za-z0-9._+-]+")
SUMS = "SHA256SUMS.txt"
TITLE_ID = "PPSA83601"
MAX_ASSET = 2 * 1024**3  # GitHub Releases 2 GiB per-asset limit
NOT_FOUND = "release not found"
REQUIRED_PIN_SHAS = ("XCL_REV", "VKD3D_REV", "PROSPERO_WIN_REV", "LAPY_REV", "WINE_REV")


class Refused(Exception):
    """The release workflow must stop: the message explains why."""


def check_version(version: str) -> str:
    if not isinstance(version, str) or not VERSION_RE.fullmatch(version):
        raise Refused(
            f"version {version!r} is not X.Y.Z or X.Y.Z.W (digits only, e.g. 0.2.21)"
        )
    return version


def version_from_ref(ref: str) -> str:
    """Return X.Y.Z from refs/heads/release/X.Y.Z (or bare release/X.Y.Z)."""
    ref = (ref or "").strip()
    if ref.startswith("refs/tags/"):
        raise Refused(
            f"{ref}: releases are built from a {BRANCH_PREFIX}X.Y.Z branch into a draft, not from a tag"
        )
    branch = ref[len("refs/heads/") :] if ref.startswith("refs/heads/") else ref
    if branch.startswith("refs/"):
        raise Refused(f"{ref}: not a branch")
    if not branch.startswith(BRANCH_PREFIX):
        raise Refused(f"{ref}: not a {BRANCH_PREFIX}X.Y.Z branch")
    version = branch[len(BRANCH_PREFIX) :]
    return check_version(version)


def content_version_for(version: str) -> str:
    """Map release version X.Y.Z to PS5 param.json contentVersion NN.NNN.NNN."""
    match = VERSION_RE.fullmatch(check_version(version))
    assert match is not None
    maj, minor, patch = (int(match.group(i)) for i in (1, 2, 3))
    return f"{max(maj, 1):02d}.{minor:03d}.{patch:03d}"


def _check_png_512(data: bytes, label: str) -> None:
    if len(data) < 24 or not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise Refused(f"{label}: not a PNG image")
    w, h = struct.unpack(">II", data[16:24])
    if (w, h) != (512, 512):
        raise Refused(f"{label}: expected 512x512 PNG, got {w}x{h}")


def check_repo(root: Path, version: str) -> None:
    """Verify release notes, sce_sys/, library TRINITYXSX_VERSION, and pins.env."""
    check_version(version)
    expected_cv = content_version_for(version)
    tag = f"v{version}"

    notes = root / "docs" / "release-notes" / f"{tag}.md"
    if not notes.is_file() or not notes.read_text(encoding="utf-8").strip():
        raise Refused(f"release notes {notes.as_posix()} are missing or empty")

    param_path = root / "sce_sys" / "param.json"
    if not param_path.is_file():
        raise Refused(f"{param_path.as_posix()}: missing")
    try:
        param = json.loads(param_path.read_text(encoding="utf-8-sig"))
    except ValueError as err:
        raise Refused(f"{param_path.as_posix()}: invalid JSON ({err})") from err
    if param.get("titleId") != TITLE_ID:
        raise Refused(f"{param_path.as_posix()}: titleId={param.get('titleId')!r}, expected {TITLE_ID!r}")
    if param.get("contentVersion") != expected_cv:
        raise Refused(
            f"{param_path.as_posix()}: contentVersion={param.get('contentVersion')!r} "
            f"does not match {tag} (expected {expected_cv!r})"
        )

    icon_path = root / "sce_sys" / "icon0.png"
    if not icon_path.is_file():
        raise Refused(f"{icon_path.as_posix()}: missing")
    _check_png_512(icon_path.read_bytes(), icon_path.as_posix())

    lib_path = root / "library" / "library.rs"
    if not lib_path.is_file():
        raise Refused(f"{lib_path.as_posix()}: missing")
    lib_text = lib_path.read_text(encoding="utf-8")
    m = re.search(r'pub\s+const\s+TRINITYXSX_VERSION\s*:\s*&str\s*=\s*"([^"]+)"\s*;', lib_text)
    if not m:
        raise Refused(f"{lib_path.as_posix()}: TRINITYXSX_VERSION constant not found")
    if m.group(1) != tag:
        raise Refused(
            f"{lib_path.as_posix()}: TRINITYXSX_VERSION={m.group(1)!r} does not match {tag!r}"
        )

    pins_path = root / "pins.env"
    if not pins_path.is_file():
        raise Refused(f"{pins_path.as_posix()}: missing")
    pins: dict[str, str] = {}
    for line in pins_path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if "=" in line:
            k, v = line.split("=", 1)
            pins[k.strip()] = v.strip()
    for key in REQUIRED_PIN_SHAS:
        val = pins.get(key, "")
        if not SHA_RE.fullmatch(val):
            raise Refused(f"{pins_path.as_posix()}: {key}={val!r} is not a 40-hex commit SHA")


def check_zip_sce_sys(zip_path: Path, sce_sys_dir: Path) -> None:
    """Verify PPSA83601.zip carries the same title metadata and icon0.png as repo sce_sys/."""
    if not zip_path.is_file():
        raise Refused(f"{zip_path}: missing")
    repo_param = json.loads((sce_sys_dir / "param.json").read_text(encoding="utf-8-sig"))
    repo_icon = (sce_sys_dir / "icon0.png").read_bytes()
    with zipfile.ZipFile(zip_path, "r") as zf:
        param_member = f"{TITLE_ID}/sce_sys/param.json"
        icon_member = f"{TITLE_ID}/sce_sys/icon0.png"
        for member in (param_member, icon_member):
            if member not in zf.namelist():
                raise Refused(f"{zip_path}: missing {member}")
        zip_param = json.loads(zf.read(param_member).decode("utf-8-sig"))
        zip_icon = zf.read(icon_member)
    for field in ("titleId", "contentVersion"):
        if zip_param.get(field) != repo_param.get(field):
            raise Refused(
                f"{zip_path}: sce_sys/param.json {field}={zip_param.get(field)!r} "
                f"differs from repo sce_sys/param.json ({repo_param.get(field)!r})"
            )
    if zip_icon != repo_icon:
        raise Refused(f"{zip_path}: sce_sys/icon0.png differs from repo sce_sys/icon0.png")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def asset_files(folder: Path) -> list[str]:
    """Return sorted regular asset filenames in a flat folder (excluding SHA256SUMS.txt)."""
    folder = Path(folder)
    if not folder.is_dir():
        raise Refused(f"{folder}: no such folder")
    names: list[str] = []
    for entry in sorted(folder.iterdir(), key=lambda p: p.name):
        if entry.name == SUMS:
            continue
        if entry.is_symlink() or not entry.is_file():
            raise Refused(f"{entry.name}: assets must be regular files in one flat folder")
        if not ASSET_NAME_RE.fullmatch(entry.name):
            raise Refused(f"{entry.name!r}: asset names may use only A-Z a-z 0-9 . _ + -")
        size = entry.stat().st_size
        if size == 0:
            raise Refused(f"{entry.name}: empty file")
        if size >= MAX_ASSET:
            raise Refused(f"{entry.name}: {size} bytes, over the 2 GiB GitHub asset limit")
        names.append(entry.name)
    if not names:
        raise Refused(f"{folder}: no assets")
    return names


def write_sums(folder: Path) -> list[str]:
    folder = Path(folder)
    lines = [f"{sha256_file(folder / name)}  {name}\n" for name in asset_files(folder)]
    (folder / SUMS).write_bytes("".join(lines).encode("ascii"))
    return lines


def verify_sums(folder: Path) -> list[str]:
    folder = Path(folder)
    sums_path = folder / SUMS
    if not sums_path.is_file():
        raise Refused(f"{folder}: no {SUMS}")
    try:
        text = sums_path.read_bytes().decode("ascii")
    except UnicodeDecodeError as err:
        raise Refused(f"{SUMS} is not ASCII") from err
    if not text.endswith("\n"):
        raise Refused(f"{SUMS} does not end with a newline")
    listed: dict[str, str] = {}
    for number, line in enumerate(text[:-1].split("\n"), 1):
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._+-]+)", line)
        if not match:
            raise Refused(f'{SUMS}:{number}: not "<sha256>  <name>"')
        digest, name = match.groups()
        if name == SUMS:
            raise Refused(f"{SUMS} lists itself")
        if name in listed:
            raise Refused(f"{SUMS} lists {name} twice")
        listed[name] = digest
    present = asset_files(folder)
    missing = sorted(set(present) - set(listed))
    extra = sorted(set(listed) - set(present))
    if missing or extra:
        raise Refused(f"{SUMS} does not match the folder: not listed {missing}, no such file {extra}")
    for name in present:
        if sha256_file(folder / name) != listed[name]:
            raise Refused(f"{name}: SHA-256 differs from {SUMS}")
    return present


def remote_tag_commit(ls_remote: str, tag: str) -> str | None:
    """Return the commit a tag names in `git ls-remote --tags` output (peeled if annotated)."""
    direct = peeled = None
    for line in (ls_remote or "").splitlines():
        parts = line.split()
        if len(parts) != 2:
            continue
        sha, ref = parts
        if ref == f"refs/tags/{tag}^{{}}":
            peeled = sha
        elif ref == f"refs/tags/{tag}":
            direct = sha
    return peeled or direct


def decide(
    tag: str,
    sha: str,
    view_exit: int,
    view_stdout: str,
    view_stderr: str,
    ls_remote: str = "",
) -> str:
    """Return 'create' or 'update'; raise Refused when the release must not be touched."""
    if not tag.startswith("v"):
        raise Refused(f"tag {tag!r} does not start with v")
    check_version(tag[1:])
    if not isinstance(sha, str) or not SHA_RE.fullmatch(sha):
        raise Refused(f"{sha!r} is not a full commit SHA")
    tagged = remote_tag_commit(ls_remote, tag)
    if tagged is not None and tagged != sha:
        raise Refused(
            f"tag {tag} already exists at {tagged}, not at {sha}; "
            "move or delete it by hand if that is intended"
        )
    if view_exit != 0:
        if NOT_FOUND in (view_stderr or "").lower():
            return "create"
        raise Refused(f"gh release view {tag} failed (exit {view_exit}): {(view_stderr or '').strip()}")
    try:
        release = json.loads(view_stdout)
    except ValueError as err:
        raise Refused(f"gh release view {tag}: unreadable answer ({err})") from err
    if not isinstance(release, dict) or release.get("tagName") != tag:
        raise Refused(f"gh release view {tag} answered for another release: {release!r}"[:300])
    if release.get("isDraft") is not True:
        raise Refused(f"release {tag} is published (not a draft); this workflow never changes it")
    if release.get("isImmutable") is True:
        raise Refused(f"release {tag} is immutable")
    return "update"


def stale_assets(release: dict, folder: Path) -> list[str]:
    present = set(asset_files(folder)) | {SUMS}
    return sorted(a["name"] for a in release.get("assets", []) if a.get("name") not in present)


def _read(path: str | None) -> str:
    return Path(path).read_text(encoding="utf-8") if path else ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("version")
    p.add_argument("--ref", required=True)

    p = sub.add_parser("content-version")
    p.add_argument("--version", required=True)

    p = sub.add_parser("check-repo")
    p.add_argument("--version", required=True)
    p.add_argument("--root", type=Path, default=Path("."))

    p = sub.add_parser("check-zip-sce-sys")
    p.add_argument("--zip", dest="zip_path", required=True, type=Path)
    p.add_argument("--sce-sys", dest="sce_sys", required=True, type=Path)

    for name in ("sums", "verify"):
        sub.add_parser(name).add_argument("folder", type=Path)

    p = sub.add_parser("decide")
    p.add_argument("--tag", required=True)
    p.add_argument("--sha", required=True)
    p.add_argument("--view-exit", required=True, type=int)
    p.add_argument("--view-stdout", required=True)
    p.add_argument("--view-stderr", required=True)
    p.add_argument("--remote-tag")

    p = sub.add_parser("stale")
    p.add_argument("folder", type=Path)
    p.add_argument("--view-stdout", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "version":
            print(version_from_ref(args.ref))
        elif args.command == "content-version":
            print(content_version_for(args.version))
        elif args.command == "check-repo":
            check_repo(args.root, args.version)
            print(f"repo metadata OK for v{args.version} (contentVersion={content_version_for(args.version)})")
        elif args.command == "check-zip-sce-sys":
            check_zip_sce_sys(args.zip_path, args.sce_sys)
            print(f"{args.zip_path.name}: sce_sys matches {args.sce_sys}")
        elif args.command == "sums":
            sys.stdout.write("".join(write_sums(args.folder)))
        elif args.command == "verify":
            print(f"{SUMS}: {len(verify_sums(args.folder))} assets match")
        elif args.command == "decide":
            print(
                decide(
                    args.tag,
                    args.sha,
                    args.view_exit,
                    _read(args.view_stdout),
                    _read(args.view_stderr),
                    _read(args.remote_tag),
                )
            )
        elif args.command == "stale":
            for name in stale_assets(json.loads(_read(args.view_stdout)), args.folder):
                print(name)
    except Refused as error:
        print(f"release_guard: refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
