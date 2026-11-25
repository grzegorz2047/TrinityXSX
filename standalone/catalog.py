# SPDX-License-Identifier: MIT
"""Read metadata from unpacked Xbox Series game folders without running them."""

import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path, PureWindowsPath
import re
from xml.etree import ElementTree as ET


MAX_MANIFEST_BYTES = 256 * 1024
MANIFEST_NAME = "microsoftgame.config"
IDENTITY = re.compile(r"[A-Za-z0-9][A-Za-z0-9.-]{0,49}")


@dataclass(frozen=True)
class Game:
    directory: Path
    identity: str
    version: str
    title_id: str | None
    name: str
    executable: str
    logo: str | None
    splash: str | None


class ManifestError(ValueError):
    pass


class ManifestTree(ET.TreeBuilder):
    def doctype(self, name, pubid, system):
        raise ManifestError("Document type declarations are unsupported")


def resolve_asset(folder: Path, value: str) -> Path | None:
    """Resolve the relative Windows path case-insensitively within the game."""
    path = PureWindowsPath(value)
    if path.anchor or not path.parts:
        raise ManifestError(f"Expected a relative asset path: {value!r}")
    candidate = folder
    for part in path.parts:
        if part == ".." or any(c in part for c in ':*?"<>|'):
            raise ManifestError(f"Invalid asset path: {value!r}")
        if not candidate.is_dir():
            return None
        matches = [item for item in candidate.iterdir() if item.name.casefold() == part.casefold()]
        if len(matches) > 1:
            raise ManifestError(f"Ambiguous asset path: {value!r}")
        if not matches:
            return None
        candidate = matches[0]
        if not candidate.resolve().is_relative_to(folder):
            raise ManifestError(f"Asset leaves the game folder: {value!r}")
    return candidate if candidate.is_file() else None


def read_game(folder: Path) -> Game:
    folder = folder.resolve(strict=True)
    manifest = resolve_asset(folder, MANIFEST_NAME)
    if manifest is None:
        raise ManifestError("Missing MicrosoftGame.config")
    with manifest.open("rb") as stream:
        data = stream.read(MAX_MANIFEST_BYTES + 1)
    if len(data) > MAX_MANIFEST_BYTES:
        raise ManifestError("MicrosoftGame.config is too large")
    try:
        root = ET.fromstring(data, parser=ET.XMLParser(target=ManifestTree()))
    except ET.ParseError as error:
        raise ManifestError(f"Invalid MicrosoftGame.config: {error}") from error
    if root.tag != "Game":
        raise ManifestError("Expected a Game element")

    identity = root.find("Identity")
    key = identity.get("Name", "") if identity is not None else ""
    if not IDENTITY.fullmatch(key) or key.endswith("."):
        raise ManifestError("Invalid or missing game identity")
    version = identity.get("Version", "")
    title_id = root.findtext("TitleId", "").strip().upper() or None
    if title_id and not re.fullmatch(r"[0-9A-F]{8}", title_id):
        raise ManifestError("TitleId must be eight hexadecimal digits")

    executables = [exe for exe in root.findall("./ExecutableList/Executable")
                   if exe.get("TargetDeviceFamily", "").casefold() == "scarlett"
                   and exe.get("IsDevOnly", "").casefold() not in {"true", "1"}]
    preferred = [exe for exe in executables if exe.get("Id", "").casefold() == "game"]
    choices = preferred or executables
    if len(choices) != 1:
        raise ManifestError("Expected one unambiguous Scarlett game executable")
    executable = resolve_asset(folder, choices[0].get("Name", ""))
    if executable is None or executable.suffix.lower() != ".exe":
        raise ManifestError("The Scarlett executable is missing or is not an .exe file")

    shell = root.find("ShellVisuals")
    attrs = shell.attrib if shell is not None else {}
    name = choices[0].get("OverrideDisplayName") or attrs.get("DefaultDisplayName") or key
    logo = choices[0].get("OverrideLogo") or attrs.get("Square150x150Logo")
    splash = choices[0].get("OverrideSplashScreenImage") or attrs.get("SplashScreenImage")

    def relative(asset: str | None) -> str | None:
        if not asset:
            return None
        file = resolve_asset(folder, asset)
        return file.relative_to(folder).as_posix() if file is not None else None

    return Game(folder, key, version, title_id, name, executable.relative_to(folder).as_posix(),
                relative(logo), relative(splash))


def scan(root: Path) -> tuple[list[Game], dict[str, str]]:
    """Read immediate child folders and retain errors without dropping valid entries."""
    games, skipped = [], {}
    for folder in sorted(root.iterdir(), key=lambda path: (path.name.casefold(), path.name)):
        if not folder.is_dir() or folder.is_symlink() or folder.is_junction():
            continue
        try:
            games.append(read_game(folder))
        except (ManifestError, OSError) as error:
            skipped[folder.name] = str(error)
    return games, skipped


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="Parent directory containing game folders")
    args = parser.parse_args()
    try:
        games, skipped = scan(args.root)
    except OSError as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps({"games": [asdict(game) for game in games], "skipped": skipped},
                     indent=2, default=str))


if __name__ == "__main__":
    main()
