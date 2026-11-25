#!/usr/bin/env python3
"""Track upstream repositories and releases listed in pins.env.

Checks latest upstream commits/releases for:
- xcl-dev/xcl (patches/xcl/*.patch)
- HansKristian-Work/vkd3d-proton + subprojects/dxil-spirv (patches/vkd3d-proton/*.patch, patches/dxil-spirv/*.patch)
- mpereiraesaa/prospero-win (patches/prospero-win/*.patch)
- mpereiraesaa/PS5-Lapy-JB-Daemon (no patches, pinned commit)
- doitsujin/dxvk (GitHub release tarball + SHA-256)
- microsoft/DirectXShaderCompiler (GitHub release zip + SHA-256)

Modes:
  python tools/sync_upstream.py --check               # dry-run check & report
  python tools/sync_upstream.py --apply               # update pins.env (and drop upstreamed patches) for clean updates
  python tools/sync_upstream.py --apply --report out.md --json out.json
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory
import urllib.request


ROOT = Path(__file__).resolve().parent.parent
PINS_PATH = ROOT / "pins.env"


@dataclass
class RepoResult:
    name: str
    old_rev: str
    new_rev: str
    status: str  # up-to-date | updated | conflict | error
    detail: str = ""
    dropped_patches: list[str] = field(default_factory=list)
    extra_updates: dict[str, str] = field(default_factory=dict)


def read_pins(path: Path) -> dict[str, str]:
    pins: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, raw_val = stripped.split("=", 1)
        val = raw_val.split("#", 1)[0].strip()
        pins[key.strip()] = val
    return pins


def update_pins_file(path: Path, updates: dict[str, str]) -> bool:
    if not updates:
        return False
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    changed = False
    new_lines: list[str] = []
    for line in lines:
        stripped = line.strip()
        if stripped and not stripped.startswith("#") and "=" in stripped:
            key = stripped.split("=", 1)[0].strip()
            if key in updates:
                # Preserve inline comments if present
                after_eq = line.split("=", 1)[1]
                comment = ""
                if "#" in after_eq:
                    idx = after_eq.index("#")
                    comment = "  " + after_eq[idx:].rstrip("\r\n")
                newline = "\r\n" if line.endswith("\r\n") else "\n"
                replacement = f"{key}={updates[key]}{comment}{newline}"
                if replacement != line:
                    changed = True
                new_lines.append(replacement)
                continue
        new_lines.append(line)
    if changed:
        path.write_text("".join(new_lines), encoding="utf-8")
    return changed


def github_api_json(url: str) -> dict:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "TrinityXSX-upstream-sync"}
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def sha256_url(url: str) -> str:
    req = urllib.request.Request(url, headers={"User-Agent": "TrinityXSX-upstream-sync"})
    h = hashlib.sha256()
    with urllib.request.urlopen(req, timeout=120) as resp:
        while chunk := resp.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def git_ls_remote_head(repo_url: str, ref: str = "HEAD") -> str:
    out = subprocess.check_output(["git", "ls-remote", repo_url, ref], text=True, timeout=30).strip()
    if not out:
        raise RuntimeError(f"git ls-remote returned empty for {repo_url} {ref}")
    return out.splitlines()[0].split()[0]


def test_patch_series_git_am(repo_dir: Path, patches: list[Path]) -> tuple[bool, list[str], str]:
    """Apply patches one by one with `git am --3way`.

    Returns (ok, dropped_patch_names, detail_message).
    """
    dropped: list[str] = []
    env = {
        **os.environ,
        "GIT_COMMITTER_NAME": "TrinityXSX",
        "GIT_COMMITTER_EMAIL": "trinityxsx@localhost",
        "GIT_AUTHOR_NAME": "TrinityXSX",
        "GIT_AUTHOR_EMAIL": "trinityxsx@localhost",
    }
    for patch in patches:
        # Check if already applied in reverse (upstream merged equivalent change)
        rev_check = subprocess.run(
            ["git", "-C", str(repo_dir), "apply", "--reverse", "--check", str(patch)],
            capture_output=True, text=True, env=env,
        )
        if rev_check.returncode == 0:
            dropped.append(patch.name)
            continue

        am = subprocess.run(
            ["git", "-C", str(repo_dir), "am", "--3way", str(patch)],
            capture_output=True, text=True, env=env,
        )
        if am.returncode != 0:
            combined = (am.stdout + "\n" + am.stderr).strip()
            subprocess.run(["git", "-C", str(repo_dir), "am", "--abort"], capture_output=True, env=env)
            if "Patch is empty" in combined or "already exists" in combined:
                dropped.append(patch.name)
                continue
            return False, dropped, f"Conflict in `{patch.name}`"
    return True, dropped, "all patches applied cleanly"


def test_patch_series_git_apply(repo_dir: Path, patches: list[Path]) -> tuple[bool, list[str], str]:
    dropped: list[str] = []
    for patch in patches:
        rev_check = subprocess.run(
            ["git", "-C", str(repo_dir), "apply", "--reverse", "--check", str(patch)],
            capture_output=True, text=True,
        )
        if rev_check.returncode == 0:
            dropped.append(patch.name)
            continue
        res = subprocess.run(
            ["git", "-C", str(repo_dir), "apply", "--3way", "--whitespace=nowarn", str(patch)],
            capture_output=True, text=True,
        )
        if res.returncode != 0:
            return False, dropped, f"Conflict in `{patch.name}`"
    return True, dropped, "all patches applied cleanly"


def check_patched_git_repo(
    name: str,
    repo_url: str,
    old_rev: str,
    patch_dir: Path,
    *,
    use_git_am: bool = True,
    submodule_rel: str | None = None,
    submodule_patch_dir: Path | None = None,
    submodule_pin_key: str | None = None,
    old_submodule_rev: str | None = None,
) -> RepoResult:
    try:
        new_rev = git_ls_remote_head(repo_url, "HEAD")
    except Exception as exc:
        return RepoResult(name, old_rev, old_rev, "error", f"fetch failed: {exc}")

    if new_rev == old_rev:
        return RepoResult(name, old_rev, new_rev, "up-to-date", "already at upstream HEAD")

    patches = sorted(patch_dir.glob("*.patch")) if patch_dir.is_dir() else []
    sub_patches = sorted(submodule_patch_dir.glob("*.patch")) if submodule_patch_dir and submodule_patch_dir.is_dir() else []

    with TemporaryDirectory(prefix=f"sync-{name}-") as tmp:
        repo_dir = Path(tmp) / name
        try:
            subprocess.run(["git", "init", "-q", str(repo_dir)], check=True)
            subprocess.run(["git", "-C", str(repo_dir), "fetch", "-q", "--depth", "50", repo_url, new_rev], check=True)
            subprocess.run(["git", "-C", str(repo_dir), "checkout", "-q", "FETCH_HEAD"], check=True)
            extra: dict[str, str] = {}
            if submodule_rel:
                subprocess.run(
                    ["git", "-C", str(repo_dir), "submodule", "update", "-q", "--init", "--depth", "1", submodule_rel],
                    check=True,
                )
                sub_dir = repo_dir / submodule_rel
                sub_rev = subprocess.check_output(["git", "-C", str(sub_dir), "rev-parse", "HEAD"], text=True).strip()
                if submodule_pin_key and sub_rev != old_submodule_rev:
                    extra[submodule_pin_key] = sub_rev
                if sub_patches:
                    sub_ok, sub_dropped, sub_msg = test_patch_series_git_am(sub_dir, sub_patches)
                    if not sub_ok:
                        return RepoResult(name, old_rev, new_rev, "conflict", f"{submodule_rel}: {sub_msg}")

            ok, dropped, detail = (
                test_patch_series_git_am(repo_dir, patches)
                if use_git_am
                else test_patch_series_git_apply(repo_dir, patches)
            )
            if not ok:
                return RepoResult(name, old_rev, new_rev, "conflict", detail)
            return RepoResult(name, old_rev, new_rev, "updated", detail, dropped_patches=dropped, extra_updates=extra)
        except Exception as exc:
            return RepoResult(name, old_rev, new_rev, "error", str(exc))


def check_dxvk(pins: dict[str, str]) -> RepoResult:
    old = pins["DXVK_VERSION"]
    try:
        rel = github_api_json("https://api.github.com/repos/doitsujin/dxvk/releases/latest")
        tag = rel["tag_name"]
        new = tag.lstrip("v")
        if new == old:
            return RepoResult("dxvk", old, new, "up-to-date", "latest release")
        asset_name = f"dxvk-{new}.tar.gz"
        url = f"https://github.com/doitsujin/dxvk/releases/download/{tag}/{asset_name}"
        digest = sha256_url(url)
        return RepoResult("dxvk", old, new, "updated", f"release {tag}", extra_updates={"DXVK_VERSION": new, "DXVK_SHA256": digest})
    except Exception as exc:
        return RepoResult("dxvk", old, old, "error", str(exc))


def check_dxc(pins: dict[str, str]) -> RepoResult:
    old_tag = pins["DXC_TAG"]
    try:
        rel = github_api_json("https://api.github.com/repos/microsoft/DirectXShaderCompiler/releases/latest")
        tag = rel["tag_name"]
        if tag == old_tag:
            return RepoResult("dxc", old_tag, tag, "up-to-date", "latest release")
        zip_asset = next(
            (a for a in rel.get("assets", []) if re.fullmatch(r"dxc_\d{4}_\d{2}_\d{2}\.zip", a["name"])),
            None,
        )
        if not zip_asset:
            return RepoResult("dxc", old_tag, tag, "error", f"no dxc_YYYY_MM_DD.zip asset in {tag}")
        digest = sha256_url(zip_asset["browser_download_url"])
        return RepoResult(
            "dxc", old_tag, tag, "updated", f"release {tag} ({zip_asset['name']})",
            extra_updates={"DXC_TAG": tag, "DXC_ZIP": zip_asset["name"], "DXC_SHA256": digest},
        )
    except Exception as exc:
        return RepoResult("dxc", old_tag, old_tag, "error", str(exc))


def check_lapy(pins: dict[str, str]) -> RepoResult:
    old = pins.get("LAPY_REV", "")
    repo = pins.get("LAPY_REPO", "")
    if not old or not repo:
        return RepoResult("lapy", old, old, "up-to-date", "not pinned")
    try:
        new = git_ls_remote_head(repo, "HEAD")
        if new == old:
            return RepoResult("lapy", old, new, "up-to-date", "already at upstream HEAD")
        return RepoResult("lapy", old, new, "updated", "new upstream commit", extra_updates={"LAPY_REV": new})
    except Exception as exc:
        return RepoResult("lapy", old, old, "error", str(exc))


def check_prospero_win_release(pins: dict[str, str]) -> RepoResult:
    old_rev = pins.get("PROSPERO_WIN_REV", "")
    old_tag = pins.get("PROSPERO_WIN_TAG", "")
    repo = pins.get("PROSPERO_WIN_REPO", "")
    if not old_rev or not repo:
        return RepoResult("prospero-win", old_rev, old_rev, "up-to-date", "not pinned")
    try:
        rel = github_api_json("https://api.github.com/repos/mpereiraesaa/prospero-win/releases/latest")
        tag = rel["tag_name"]
        if tag == old_tag:
            return RepoResult("prospero-win", old_tag, tag, "up-to-date", f"latest release ({old_rev[:10]})")
        tag_rev = git_ls_remote_head(repo, f"refs/tags/{tag}^{{}}") if False else None
        # Resolve annotated or lightweight tag commit
        out = subprocess.check_output(["git", "ls-remote", repo, f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"], text=True, timeout=30)
        lines = [l.split() for l in out.strip().splitlines() if l.strip()]
        peeled = [sha for sha, ref in lines if ref.endswith("^{}")]
        new_rev = peeled[0] if peeled else lines[0][0]
        # Test patches/prospero-win/*.patch against the new release tag
        with TemporaryDirectory(prefix="sync-pw-") as tmp:
            repo_dir = Path(tmp) / "prospero-win"
            subprocess.run(["git", "init", "-q", str(repo_dir)], check=True)
            subprocess.run(["git", "-C", str(repo_dir), "fetch", "-q", "--depth", "1", repo, new_rev], check=True)
            subprocess.run(["git", "-C", str(repo_dir), "checkout", "-q", "FETCH_HEAD"], check=True)
            patches = sorted((ROOT / "patches/prospero-win").glob("*.patch"))
            ok, dropped, detail = test_patch_series_git_apply(repo_dir, patches)
            if not ok:
                return RepoResult("prospero-win", f"{old_tag} ({old_rev[:8]})", f"{tag} ({new_rev[:8]})", "conflict", detail)
            extra = {"PROSPERO_WIN_TAG": tag, "PROSPERO_WIN_REV": new_rev}
            zip_asset = next((a for a in rel.get("assets", []) if a["name"].endswith(".zip")), None)
            if zip_asset:
                extra["PROSPERO_WIN_ZIP_SHA256"] = sha256_url(zip_asset["browser_download_url"])
            return RepoResult(
                "prospero-win", f"{old_tag} ({old_rev[:8]})", f"{tag} ({new_rev[:8]})",
                "updated", f"{detail}; rebuild runner/wineserver before cutting standalone .ffpkg",
                dropped_patches=dropped, extra_updates=extra,
            )
    except Exception as exc:
        return RepoResult("prospero-win", old_tag, old_tag, "error", str(exc))


def render_markdown(results: list[RepoResult], applied: bool) -> str:
    lines = [
        "# Upstream Sync Report",
        "",
        "| Dependency | Current | Upstream | Status | Details |",
        "|---|---|---|---|---|",
    ]
    for r in results:
        old_short = r.old_rev[:10] if len(r.old_rev) == 40 else r.old_rev
        new_short = r.new_rev[:10] if len(r.new_rev) == 40 else r.new_rev
        detail = r.detail
        if r.dropped_patches:
            detail += f" (upstreamed/dropped: {', '.join(r.dropped_patches)})"
        lines.append(f"| `{r.name}` | `{old_short}` | `{new_short}` | **{r.status}** | {detail} |")
    lines.append("")
    if applied:
        lines.append("`pins.env` was updated for all clean (`updated`) entries; conflicting entries were kept at their current pins.")
    else:
        lines.append("Dry run (`--check`). Run `python tools/sync_upstream.py --apply` to update `pins.env`.")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Update pins.env and remove upstreamed patches for clean updates")
    parser.add_argument("--check", action="store_true", help="Dry run (default)")
    parser.add_argument("--only", choices=("xcl", "vkd3d", "dxvk", "dxc", "prospero-win", "lapy"), help="Check a single dependency")
    parser.add_argument("--report", type=Path, help="Write Markdown report to file")
    parser.add_argument("--json", type=Path, dest="json_out", help="Write JSON report to file")
    args = parser.parse_args()

    pins = read_pins(PINS_PATH)
    results: list[RepoResult] = []
    pin_updates: dict[str, str] = {}
    patches_to_drop: list[Path] = []

    def want(name: str) -> bool:
        return args.only is None or args.only == name

    if want("xcl"):
        r = check_patched_git_repo("xcl", pins["XCL_REPO"], pins["XCL_REV"], ROOT / "patches/xcl", use_git_am=True)
        results.append(r)
        if r.status == "updated":
            pin_updates["XCL_REV"] = r.new_rev
            patches_to_drop.extend((ROOT / "patches/xcl" / p) for p in r.dropped_patches)

    if want("vkd3d"):
        r = check_patched_git_repo(
            "vkd3d-proton", pins["VKD3D_REPO"], pins["VKD3D_REV"], ROOT / "patches/vkd3d-proton",
            use_git_am=True,
            submodule_rel="subprojects/dxil-spirv",
            submodule_patch_dir=ROOT / "patches/dxil-spirv",
            submodule_pin_key="DXIL_SPIRV_REV",
            old_submodule_rev=pins.get("DXIL_SPIRV_REV"),
        )
        results.append(r)
        if r.status == "updated":
            pin_updates["VKD3D_REV"] = r.new_rev
            pin_updates.update(r.extra_updates)
            patches_to_drop.extend((ROOT / "patches/vkd3d-proton" / p) for p in r.dropped_patches)

    if want("dxvk"):
        r = check_dxvk(pins)
        results.append(r)
        if r.status == "updated":
            pin_updates.update(r.extra_updates)

    if want("dxc"):
        r = check_dxc(pins)
        results.append(r)
        if r.status == "updated":
            pin_updates.update(r.extra_updates)

    if want("prospero-win"):
        r = check_prospero_win_release(pins)
        results.append(r)
        if r.status == "updated":
            pin_updates.update(r.extra_updates)

    if want("lapy"):
        r = check_lapy(pins)
        results.append(r)
        if r.status == "updated":
            pin_updates.update(r.extra_updates)

    applied = False
    if args.apply and (pin_updates or patches_to_drop):
        applied = update_pins_file(PINS_PATH, pin_updates)
        for patch_path in patches_to_drop:
            if patch_path.exists():
                patch_path.unlink()
                applied = True

    md = render_markdown(results, applied=applied)
    print(md)
    if args.report:
        args.report.write_text(md, encoding="utf-8")
    if args.json_out:
        payload = {
            "applied": applied,
            "updated": [r.name for r in results if r.status == "updated"],
            "conflicts": [r.name for r in results if r.status == "conflict"],
            "errors": [r.name for r in results if r.status == "error"],
            "results": [r.__dict__ for r in results],
        }
        args.json_out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
