# SPDX-License-Identifier: MIT

import json
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
import unittest
import zipfile

from .catalog import ManifestError, read_game, scan
from .image import write_app_zip
from .native.build_prefix_seed import CONSOLE_CPU, CPU_KEY, PC_CPU, rewrite_system_reg, sanitize_symlinks
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
        stamp1 = (library_output / "prefix/drive_c/XCL/runtime-stamp.txt").read_text(encoding="ascii").strip()
        self.assertEqual(len(stamp1), 64)
        self.assertFalse((runner / RUNNER_EXECUTABLE).exists())

        library.write_bytes(b"MZ TrinityXSX: library window opened v2")
        library_output_v2 = self.root / "library_v2/PPSA83601"
        stage(None, runtime, library_output_v2, runner, seed, library_exe=library, launcher=launcher)
        stamp2 = (library_output_v2 / "prefix/drive_c/XCL/runtime-stamp.txt").read_text(encoding="ascii").strip()
        self.assertEqual(len(stamp2), 64)
        self.assertNotEqual(stamp1, stamp2)

    def test_prefix_seed_registry_rewrite_preserves_other_sections(self):
        reg = (
            b"WINE REGISTRY Version 2\n\n"
            b"[Software\\\\Microsoft\\\\Other]\n" + PC_CPU + b"\n\n"
            + CPU_KEY + b"\n" + PC_CPU + b"\n\n"
            b"[Software\\\\Trailing]\n@=\"keep\"\n"
        )
        rewritten = rewrite_system_reg(reg)
        self.assertIn(CPU_KEY + b"\n" + CONSOLE_CPU, rewritten)
        self.assertIn(b"[Software\\\\Microsoft\\\\Other]\n" + PC_CPU, rewritten)
        self.assertIn(b"[Software\\\\Trailing]\n@=\"keep\"", rewritten)
        sanitize_symlinks(self.root / "games")

    def test_write_app_zip_preserves_top_level_title_id_and_exec_bits(self):
        app = self.root / "staged"
        (app / "sce_sys").mkdir(parents=True)
        (app / "runtime").mkdir()
        (app / "eboot.bin").write_bytes(b"\x7fELF")
        (app / RUNNER_EXECUTABLE).write_bytes(b"\x7fELF")
        (app / "sce_sys/param.json").write_text('{"titleId":"PPSA83601"}', encoding="utf-8")
        (app / "sce_sys/icon0.png").write_bytes(b"\x89PNG")
        out_zip = self.root / "out/PPSA83601.zip"
        digest = write_app_zip(app, out_zip)
        self.assertEqual(len(digest), 64)
        with zipfile.ZipFile(out_zip, "r") as zf:
            names = set(zf.namelist())
            self.assertIn("PPSA83601/eboot.bin", names)
            self.assertIn("PPSA83601/sce_sys/param.json", names)
            self.assertIn("PPSA83601/sce_sys/icon0.png", names)
            eboot_mode = (zf.getinfo("PPSA83601/eboot.bin").external_attr >> 16) & 0o777
            param_mode = (zf.getinfo("PPSA83601/sce_sys/param.json").external_attr >> 16) & 0o777
            self.assertEqual(eboot_mode, 0o755)
            self.assertEqual(param_mode, 0o644)

    def test_native_prefix_seed_sync_runtime_updates_binaries_and_keeps_user_data(self):
        if os.name != "posix":
            self.skipTest("POSIX C11 host test for prefix_seed.c runs on Linux/CI")
        cc = shutil.which("clang") or shutil.which("gcc") or shutil.which("cc")
        if not cc:
            self.skipTest("No C compiler available on host")
        native_dir = Path(__file__).with_name("native")
        harness = self.root / "test_sync.c"
        binary = self.root / "test_sync"
        harness.write_text(
            """
            #include <assert.h>
            #include <stdio.h>
            #include <string.h>
            int trinityxsx_prepare_storage(const char *source, const char *parent, const char *root, const char **step);
            int trinityxsx_sync_runtime(const char *source, const char *root, const char **step);
            int main(int argc, char **argv) {
                assert(argc == 4);
                const char *step = "none";
                if (strcmp(argv[1], "prepare") == 0) {
                    int rc = trinityxsx_prepare_storage(argv[2], argv[3], argv[3], &step);
                    if (rc != 0) { fprintf(stderr, "prepare failed at %s\\n", step); return 1; }
                    return 0;
                }
                int rc = trinityxsx_sync_runtime(argv[2], argv[3], &step);
                printf("%d\\n", rc);
                if (rc < 0) { fprintf(stderr, "sync failed at %s\\n", step); return 1; }
                return 0;
            }
            """,
            encoding="utf-8",
        )
        subprocess.run(
            [cc, "-std=c11", "-D_POSIX_C_SOURCE=200809L", "-O2", "-Wall", "-Wextra", "-Werror",
             str(native_dir / "prefix_seed.c"), str(harness), "-o", str(binary)],
            check=True,
        )
        # Also syntax-check launcher.c with -DTRINITYXSX_WITH_SEED -Wall -Wextra -Werror
        subprocess.run(
            [cc, "-std=c11", "-D_POSIX_C_SOURCE=200809L", "-O2", "-Wall", "-Wextra", "-Werror", "-DTRINITYXSX_WITH_SEED",
             "-c", str(native_dir / "launcher.c"), "-o", str(self.root / "launcher.o")],
            check=True,
        )

        seed = self.root / "seed"
        dest = self.root / "dest/PPSA83601"
        (seed / "prefix/drive_c/XCL/games").mkdir(parents=True)
        (seed / "profiles").mkdir(parents=True)
        (seed / "prefix/drive_c/XCL/xcl-library.exe").write_bytes(b"v2-new-exe")
        (seed / "prefix/drive_c/XCL/d3d12core.dll").write_bytes(b"v2-new-dll")
        (seed / "prefix/drive_c/XCL/runtime-stamp.txt").write_text("stamp-v2\n", encoding="ascii")
        (seed / "profiles/profiles.lst").write_text("trinityxsx-library.profile\n", encoding="ascii")
        (seed / "profiles/trinityxsx-library.profile").write_text("default-profile\n", encoding="ascii")

        # Existing v1 install with user saves, framegen profile, game_paths, and user game
        (dest / "prefix/drive_c/XCL/games/mygame").mkdir(parents=True)
        (dest / "prefix/drive_c/users/prospero/saves").mkdir(parents=True)
        (dest / "profiles").mkdir(parents=True)
        (dest / "prefix/drive_c/XCL/xcl-library.exe").write_bytes(b"v1-old-exe")
        (dest / "prefix/drive_c/XCL/d3d12core.dll").write_bytes(b"v1-old-dll")
        (dest / "prefix/drive_c/XCL/games/mygame/game.exe").write_bytes(b"user-game")
        (dest / "prefix/drive_c/users/prospero/saves/world.dat").write_bytes(b"precious-save")
        (dest / "profiles/trinityxsx-library.profile").write_text("framegen = 2\n", encoding="ascii")
        (dest / "game_paths.txt").write_text("/mnt/ext1/xboxseries\n", encoding="ascii")

        # First sync: missing stamp -> upgrades runtime binaries, preserves all user data!
        r1 = subprocess.run([str(binary), "sync", str(seed), str(dest)], capture_output=True, text=True, check=True)
        self.assertEqual(r1.stdout.strip(), "1")
        self.assertEqual((dest / "prefix/drive_c/XCL/xcl-library.exe").read_bytes(), b"v2-new-exe")
        self.assertEqual((dest / "prefix/drive_c/XCL/d3d12core.dll").read_bytes(), b"v2-new-dll")
        self.assertEqual((dest / "prefix/drive_c/XCL/runtime-stamp.txt").read_text(), "stamp-v2\n")
        self.assertEqual((dest / "prefix/drive_c/XCL/games/mygame/game.exe").read_bytes(), b"user-game")
        self.assertEqual((dest / "prefix/drive_c/users/prospero/saves/world.dat").read_bytes(), b"precious-save")
        self.assertEqual((dest / "profiles/trinityxsx-library.profile").read_text(), "framegen = 2\n")
        self.assertEqual((dest / "game_paths.txt").read_text(), "/mnt/ext1/xboxseries\n")

        # Second sync: stamp matches -> no-op (returns 0)
        r2 = subprocess.run([str(binary), "sync", str(seed), str(dest)], capture_output=True, text=True, check=True)
        self.assertEqual(r2.stdout.strip(), "0")


if __name__ == "__main__":
    unittest.main()
