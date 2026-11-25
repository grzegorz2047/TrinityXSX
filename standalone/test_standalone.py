# SPDX-License-Identifier: MIT

import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from .catalog import ManifestError, read_game, scan
from .stage import RUNNER_EXECUTABLE, RUNTIME_FILES, profile_text, stage


MANIFEST = """<Game>
  <Identity Name="Microsoft.MinecraftBeta" Version="1.18.3064.0" />
  <TitleId>639AA8DD</TitleId>
  <ShellVisuals DefaultDisplayName="Minecraft &amp; Preview" Square150x150Logo="logo.png" />
  <ExecutableList>
    <Executable Id="Game" Name="desktop.exe" TargetDeviceFamily="XboxOne" />
    <Executable Id="Tools" Name="dev.exe" TargetDeviceFamily="Scarlett" IsDevOnly="true" />
    <Executable Id="Game" Name="minecraft.windows.exe" TargetDeviceFamily="Scarlett" />
  </ExecutableList>
</Game>"""


class StandaloneTests(unittest.TestCase):
    def setUp(self):
        temp = TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.games = self.root / "games"
        self.game = self.games / "Minecraft"
        self.game.mkdir(parents=True)
        (self.game / "Minecraft.Windows.exe").write_bytes(b"test executable")
        (self.game / "Logo.PNG").write_bytes(b"test logo")
        self.manifest = self.game / "MicrosoftGame.Config"
        self.manifest.write_text(MANIFEST, encoding="utf-8")

    def test_catalog_selects_scarlett_and_preserves_usable_asset_case(self):
        (self.games / "broken").mkdir()
        games, skipped = scan(self.games)
        self.assertEqual(len(games), 1)
        self.assertEqual(games[0].executable, "Minecraft.Windows.exe")
        self.assertEqual(games[0].logo, "Logo.PNG")
        self.assertEqual(games[0].name, "Minecraft & Preview")
        self.assertIn("broken", skipped)

    def test_outside_game_and_document_types_are_rejected(self):
        self.manifest.write_text(MANIFEST.replace("logo.png", "../logo.png"), encoding="utf-8")
        with self.assertRaisesRegex(ManifestError, "Invalid asset path"):
            read_game(self.game)
        self.manifest.write_text("<!DOCTYPE Game>\n" + MANIFEST, encoding="utf-8")
        with self.assertRaisesRegex(ManifestError, "Document type"):
            read_game(self.game)

    def test_profile_cannot_add_fields_through_game_name(self):
        with self.assertRaises(ValueError):
            profile_text("Minecraft\nexecutable = other.exe")

    def test_full_stage_keeps_seed_and_profiles_isolated(self):
        runtime = self.root / "runtime"
        runner = self.root / "runner"
        seed = self.root / "seed"
        runtime.mkdir()
        runner.mkdir()
        seed.mkdir()
        (runtime / "licenses").mkdir()
        for name in RUNTIME_FILES:
            (runtime / name).write_bytes(b"test runtime file")
        for name in ("eboot.bin", "sce_sys/icon0.png", "sce_module/libc.prx", "SOURCES.txt",
                     "win/wine/lib/wine/x86_64-unix/ntdll.prx",
                     "win/wine/lib/wine/x86_64-unix/wineps5.prx"):
            entry = runner / name
            entry.parent.mkdir(parents=True, exist_ok=True)
            entry.write_bytes(b"test runner file")
        param = runner / "sce_sys/param.json"
        param.write_text('{"titleId":"PPSA99995"}', encoding="utf-8")
        for name in ("system.reg", "user.reg", "userdef.reg"):
            (seed / name).write_text("test registry", encoding="utf-8")
        (seed / "drive_c/windows").mkdir(parents=True)
        (seed / "dosdevices").mkdir()
        (seed / "dosdevices/.pw-symlinks").write_text("c:\t../drive_c\n", encoding="utf-8")

        output = self.root / "PPSA83601"
        with self.assertRaisesRegex(ValueError, "already be built for PPSA83601"):
            stage(self.game, runtime, output, runner, seed)
        self.assertFalse(output.exists())

        param.write_text('{"titleId":"PPSA83601"}', encoding="utf-8")
        stage(self.game, runtime, output, runner, seed)
        record = json.loads((output / "TRINITYXSX-BUILD.json").read_text(encoding="utf-8"))
        self.assertTrue(record["contains_native_runner_and_prefix_seed"])
        self.assertEqual((output / "prefix/system.reg").read_text(), "test registry")
        self.assertEqual((output / "profiles/profiles.lst").read_text(), "minecraft-beta.profile\n")
        self.assertEqual(list((output / record["game_install_path"]).iterdir()), [])
        self.assertFalse((seed / "drive_c/XCL").exists())
        with self.assertRaises(FileExistsError):
            stage(self.game, runtime, output, runner, seed)

        library = self.root / "xcl-library.exe"
        library.write_bytes(b"MZ TrinityXSX: library window opened")
        launcher = self.root / "launcher.bin"
        launcher.write_bytes(b"\x7fELF native launcher fixture")
        library_output = self.root / "library/PPSA83601"
        with self.assertRaisesRegex(ValueError, "native ELF or SELF"):
            stage(None, runtime, library_output, runner, seed, library_exe=library, launcher=library)
        self.assertFalse(library_output.exists())

        stage(None, runtime, library_output, runner, seed, library_exe=library, launcher=launcher)
        self.assertEqual((library_output / "eboot.bin").read_bytes(), launcher.read_bytes())
        self.assertEqual((library_output / RUNNER_EXECUTABLE).read_bytes(), (runner / "eboot.bin").read_bytes())
        metadata = json.loads((library_output / "sce_sys/param.json").read_text(encoding="utf-8"))
        self.assertEqual(metadata["localizedParameters"]["en-US"]["titleName"], "TrinityXSX")
        icon = Path(__file__).with_name("assets") / "icon0.png"
        self.assertEqual((library_output / "sce_sys/icon0.png").read_bytes(), icon.read_bytes())
        self.assertEqual((library_output / "profiles/profiles.lst").read_text(), "trinityxsx-library.profile\n")
        self.assertFalse((runner / RUNNER_EXECUTABLE).exists())


if __name__ == "__main__":
    unittest.main()
