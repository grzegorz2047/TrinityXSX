# SPDX-License-Identifier: MIT
"""Unit tests for tools/release_guard.py."""

from __future__ import annotations

import json
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

from tools import release_guard as rg

SHA_A = "a" * 40
SHA_B = "b" * 40
PNG_512 = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR" + struct.pack(">II", 512, 512) + b"\x08\x06\x00\x00\x00"


def make_repo(root: Path, version: str = "0.2.21", cv: str = "01.002.021") -> None:
    (root / "docs" / "release-notes").mkdir(parents=True)
    (root / "docs" / "release-notes" / f"v{version}.md").write_text(f"## TrinityXSX v{version}\n", encoding="utf-8")
    (root / "sce_sys").mkdir()
    (root / "sce_sys" / "param.json").write_text(
        json.dumps({"titleId": "PPSA83601", "contentVersion": cv}),
        encoding="utf-8",
    )
    (root / "sce_sys" / "icon0.png").write_bytes(PNG_512)
    (root / "library").mkdir()
    (root / "library" / "library.rs").write_text(
        f'pub const TRINITYXSX_VERSION: &str = "v{version}";\n',
        encoding="utf-8",
    )
    (root / "pins.env").write_text(
        "\n".join(f"{k}={SHA_A}" for k in rg.REQUIRED_PIN_SHAS) + "\n",
        encoding="utf-8",
    )


class ReleaseGuardTests(unittest.TestCase):
    def test_version_from_ref_and_content_version(self) -> None:
        self.assertEqual(rg.version_from_ref("refs/heads/release/0.2.21"), "0.2.21")
        self.assertEqual(rg.version_from_ref("release/1.2.3.1"), "1.2.3.1")
        self.assertEqual(rg.content_version_for("0.2.21"), "01.002.021")
        self.assertEqual(rg.content_version_for("2.10.5"), "02.010.005")
        for bad in (
            "refs/tags/v0.2.21",
            "refs/heads/main",
            "refs/heads/release/v0.2.21",
            "refs/heads/release/0.2.21-rc1",
            "refs/pull/10/merge",
        ):
            with self.assertRaises(rg.Refused, msg=bad):
                rg.version_from_ref(bad)

    def test_check_repo_validates_notes_sce_sys_library_and_pins(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            make_repo(root)
            rg.check_repo(root, "0.2.21")

            # Missing release notes refused
            (root / "docs" / "release-notes" / "v0.2.21.md").write_text("   \n", encoding="utf-8")
            with self.assertRaises(rg.Refused):
                rg.check_repo(root, "0.2.21")
            (root / "docs" / "release-notes" / "v0.2.21.md").write_text("ok\n", encoding="utf-8")

            # Mismatched library version refused
            (root / "library" / "library.rs").write_text('pub const TRINITYXSX_VERSION: &str = "v0.2.20";\n')
            with self.assertRaises(rg.Refused):
                rg.check_repo(root, "0.2.21")
            (root / "library" / "library.rs").write_text('pub const TRINITYXSX_VERSION: &str = "v0.2.21";\n')

            # Mismatched param.json contentVersion refused
            (root / "sce_sys" / "param.json").write_text(
                json.dumps({"titleId": "PPSA83601", "contentVersion": "01.002.020"})
            )
            with self.assertRaises(rg.Refused):
                rg.check_repo(root, "0.2.21")

    def test_check_zip_sce_sys(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            make_repo(root)
            z = root / "PPSA83601.zip"
            with zipfile.ZipFile(z, "w") as zf:
                zf.writestr("PPSA83601/sce_sys/param.json", (root / "sce_sys" / "param.json").read_bytes())
                zf.writestr("PPSA83601/sce_sys/icon0.png", PNG_512)
            rg.check_zip_sce_sys(z, root / "sce_sys")

            with zipfile.ZipFile(z, "w") as zf:
                zf.writestr("PPSA83601/sce_sys/param.json", json.dumps({"titleId": "PPSA83601", "contentVersion": "01.002.020"}))
                zf.writestr("PPSA83601/sce_sys/icon0.png", PNG_512)
            with self.assertRaises(rg.Refused):
                rg.check_zip_sce_sys(z, root / "sce_sys")

    def test_sums_and_verify(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            folder = Path(td)
            (folder / "PPSA83601.zip").write_bytes(b"zip-bytes")
            (folder / "txin-sender-v0.2.21.exe").write_bytes(b"exe-bytes")
            lines = rg.write_sums(folder)
            self.assertEqual(len(lines), 2)
            self.assertEqual(rg.verify_sums(folder), ["PPSA83601.zip", "txin-sender-v0.2.21.exe"])

            # Tampered byte fails verification
            (folder / "PPSA83601.zip").write_bytes(b"zip-BYTEZ")
            with self.assertRaises(rg.Refused):
                rg.verify_sums(folder)

            # Extra unlisted file fails verification
            (folder / "PPSA83601.zip").write_bytes(b"zip-bytes")
            (folder / "extra.bin").write_bytes(b"x")
            with self.assertRaises(rg.Refused):
                rg.verify_sums(folder)

    def test_decide_and_stale(self) -> None:
        # Missing release -> create
        self.assertEqual(rg.decide("v0.2.21", SHA_A, 1, "", "release not found\n", ""), "create")
        # Draft release -> update
        draft_json = json.dumps(
            {
                "tagName": "v0.2.21",
                "isDraft": True,
                "isImmutable": False,
                "targetCommitish": SHA_A,
                "assets": [{"name": "PPSA83601.zip"}, {"name": "old.zip"}, {"name": "SHA256SUMS.txt"}],
            }
        )
        self.assertEqual(rg.decide("v0.2.21", SHA_A, 0, draft_json, "", ""), "update")
        # Published release -> refused
        pub_json = json.dumps({"tagName": "v0.2.21", "isDraft": False, "isImmutable": False})
        with self.assertRaises(rg.Refused):
            rg.decide("v0.2.21", SHA_A, 0, pub_json, "", "")
        # Immutable draft -> refused
        imm_json = json.dumps({"tagName": "v0.2.21", "isDraft": True, "isImmutable": True})
        with self.assertRaises(rg.Refused):
            rg.decide("v0.2.21", SHA_A, 0, imm_json, "", "")
        # Tag already exists on different commit -> refused (including annotated tag peeled ref)
        ls_remote = f"{SHA_B}\trefs/tags/v0.2.21\n{SHA_B}\trefs/tags/v0.2.21^{{}}\n"
        with self.assertRaises(rg.Refused):
            rg.decide("v0.2.21", SHA_A, 1, "", "release not found", ls_remote)
        # Unreadable gh error -> refused
        with self.assertRaises(rg.Refused):
            rg.decide("v0.2.21", SHA_A, 1, "", "HTTP 502 Bad Gateway", "")

        with tempfile.TemporaryDirectory() as td:
            folder = Path(td)
            (folder / "PPSA83601.zip").write_bytes(b"ok")
            self.assertEqual(rg.stale_assets(json.loads(draft_json), folder), ["old.zip"])


if __name__ == "__main__":
    unittest.main()
