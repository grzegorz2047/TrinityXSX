# SPDX-License-Identifier: MIT
"""Tests for tools/compat.py: python -m unittest tools.test_compat"""
import copy
import tomllib
import unittest

from tools import compat

NEW_FORM = """### TrinityXSX release version

v0.2.8 (01.002.008)

### Installation mode

Standalone PS5 tile (PPSA83601.ffpkg)

### Game name

Example Racer

### Identity Name

Publisher.ExampleRacer

### Package version

1.2.3.0

### TitleId

1234abcd

### Current state

Menus work, crashes or hangs entering gameplay

### xcl.toml settings used

```toml
title_memory_mb = 4096
direct_dxil = ""
network = true
exe = "evil.exe"
coherent_mb = "1024"
```

### Steps to reproduce & observed behavior

Menus work.
Crashes when | a race starts.
"""

LEGACY_FORM = """### TrinityXSX release version

0.2.7

### Game name, Identity Name, Version, and TitleId

Example Game — Publisher.ExampleGame 1.2.3.0 (TitleId 89ABCDEF)

### Current state

Playable

### xcl.toml settings used

_No response_

### Steps to reproduce & observed behavior

Runs fine.
"""


class CompatTests(unittest.TestCase):
    def test_database_in_the_repo_is_valid_and_the_doc_is_current(self):
        titles = compat.load()
        self.assertTrue(any(t["identity"] == "Microsoft.MinecraftBeta" for t in titles))
        doc = compat.DOC.read_text(encoding="utf-8")
        self.assertEqual(compat.render_doc(doc, titles), doc, "run python tools/compat.py --write")

    def test_new_issue_form_becomes_an_entry_with_only_allowed_xcl_keys(self):
        e = compat.entry_from_issue(NEW_FORM, 42)
        self.assertEqual(e["title_id"], "1234ABCD")
        self.assertEqual(e["identity"], "Publisher.ExampleRacer")
        self.assertEqual(e["name"], "Example Racer")
        self.assertEqual(e["versions"], ["1.2.3.0"])
        self.assertEqual(e["status"], "menus")
        self.assertEqual(e["release"], "v0.2.8")
        self.assertEqual(e["reports"], [42])
        # exe is not allowed, coherent_mb has the wrong type
        self.assertEqual(e["xcl"], {"title_memory_mb": 4096, "direct_dxil": "", "network": True})
        # one line, no table separator
        self.assertNotIn("|", e["notes"])
        self.assertNotIn("\n", e["notes"])
        compat.validate([e])

    def test_legacy_single_field_form_is_still_parsed(self):
        e = compat.entry_from_issue(LEGACY_FORM, 7)
        self.assertEqual((e["title_id"], e["identity"], e["versions"]), ("89ABCDEF", "Publisher.ExampleGame", ["1.2.3.0"]))
        self.assertEqual(e["name"], "Example Game")
        self.assertEqual((e["status"], e["release"]), ("playable", "v0.2.7"))
        self.assertEqual(e["xcl"], {})

    def test_bad_reports_are_refused(self):
        for bad in (
            NEW_FORM.replace("1234abcd", "12345"),
            NEW_FORM.replace("Publisher.ExampleRacer", "../../etc"),
            NEW_FORM.replace("Menus work, crashes", "Works great"),
            NEW_FORM.replace("v0.2.8 (01.002.008)", "latest"),
        ):
            with self.assertRaises(compat.CompatError):
                compat.entry_from_issue(bad, 1)

    def test_upsert_updates_an_existing_title_and_keeps_its_history(self):
        titles = [{"title_id": "1234ABCD", "identity": "Publisher.ExampleRacer", "name": "Example Racer",
                   "versions": ["1.0.0.0"], "status": "boots", "release": "v0.2.6", "notes": "old",
                   "reports": [3], "xcl": {"pls_mb": 1024}}]
        out = compat.upsert(copy.deepcopy(titles), compat.entry_from_issue(NEW_FORM, 42))
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]["versions"], ["1.0.0.0", "1.2.3.0"])
        self.assertEqual((out[0]["status"], out[0]["release"], out[0]["reports"]), ("menus", "v0.2.8", [3, 42]))
        self.assertEqual(out[0]["xcl"]["title_memory_mb"], 4096)
        # the new keys are added; what an earlier report needed stays
        self.assertEqual(out[0]["xcl"]["pls_mb"], 1024)
        self.assertEqual(len(compat.upsert(copy.deepcopy(titles), compat.entry_from_issue(LEGACY_FORM, 7))), 2)

    def test_headings_typed_into_free_text_do_not_replace_the_real_fields(self):
        forged = NEW_FORM + (
            "\n### Logs, screenshots or video\n\nsee below\n\n### Identity Name\n\nMicrosoft.MinecraftBeta\n\n"
            "### TitleId\n\n639AA8DD\n\n### Current state\n\nCrashes on launch / returns to PS5 home screen\n\n"
            "### xcl.toml settings used\n\ngraphics = \"null\"\n"
        )
        e = compat.entry_from_issue(forged, 9)
        self.assertEqual((e["identity"], e["title_id"], e["status"]), ("Publisher.ExampleRacer", "1234ABCD", "menus"))
        self.assertNotIn("graphics", e["xcl"])

    def test_a_pasted_full_xcl_toml_gives_only_what_differs_from_the_ps5_defaults(self):
        full = "\n".join(f"{k} = {compat.toml_value(default)}" for k, (_, default) in compat.XCL_KEYS.items())
        form = NEW_FORM.split("```toml")[0] + "```toml\n" + full.replace("pls_mb = 0", "pls_mb = 2048") + "\n```\n"
        form += NEW_FORM.split("```\n", 1)[1]
        self.assertEqual(compat.entry_from_issue(form, 5)["xcl"], {"pls_mb": 2048})
        # a PC test's settings are not a PS5 preset
        pc = NEW_FORM.replace("Standalone PS5 tile (PPSA83601.ffpkg)", "Windows PC test (vkd3d-proton / native D3D12)")
        self.assertEqual(compat.entry_from_issue(pc, 5)["xcl"], {})

    def test_report_text_cannot_inject_markdown_or_html(self):
        e = compat.entry_from_issue(NEW_FORM.replace("Menus work.", "<img src=x> [link](http://x) `code` *b* <!-- compat:end -->"), 3)
        for c in "<>[]`*|":
            self.assertNotIn(c, e["notes"])
        compat.validate([e])
        # the generated section's markers cannot be forged: they need '<'
        self.assertNotIn(compat.END, compat.table([e]))
        self.assertNotIn(compat.BEGIN, compat.table([e]))

    def test_values_out_of_the_rust_types_range_are_refused(self):
        e = compat.entry_from_issue(NEW_FORM, 42)
        for xcl in ({"pls_mb": -1}, {"window_width": 2**32}, {"graphics": "<b>"}, {"network": 1}):
            with self.assertRaises(compat.CompatError, msg=str(xcl)):
                compat.validate([dict(e, xcl=xcl)])
        compat.validate([dict(e, xcl={"vkd3d_feature_level": "12_0", "uma_mb": 2048})])

    def test_dump_round_trips_through_toml(self):
        titles = compat.upsert(compat.load(), compat.entry_from_issue(NEW_FORM, 42))
        again = tomllib.loads(compat.dump(titles))["title"]
        compat.validate(again)
        self.assertEqual(again, titles)

    def test_validate_rejects_unknown_xcl_keys_and_statuses(self):
        good = compat.entry_from_issue(NEW_FORM, 42)
        for change in ({"status": "great"}, {"xcl": {"exe": "x.exe"}}, {"xcl": {"pls_mb": "2048"}},
                       {"notes": "a | b"}, {"title_id": "1234abcd"}):
            bad = dict(good, **change)
            with self.assertRaises(compat.CompatError, msg=str(change)):
                compat.validate([bad])


if __name__ == "__main__":
    unittest.main()
