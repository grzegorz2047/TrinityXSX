# SPDX-License-Identifier: MIT
"""Tests for tools/ps5_bench.py (no PS5 needed): python -m unittest tools.test_ps5_bench"""
import json
import tempfile
import tomllib
import unittest
from pathlib import Path

from tools import ps5_bench as bench

OK = {"status": "ok", "avg_fps": 60.0, "low1_fps": 50.0, "p99_ms": 20.0, "stalls_100ms": 1,
      "first_frame_s": 10.0, "peak_commit_mb": 4000}


class BenchTests(unittest.TestCase):
    def test_request_is_valid_toml_with_per_title_environment(self):
        text = bench.request_text("20261126-120000", ["TrinityXSX.AetherDrift", "TrinityXSX.NeonVoid"], 60, 15,
                                  {"TrinityXSX.AetherDrift": {"AETHER_AUTO_DEMO": "1"}})
        data = tomllib.loads(text)
        self.assertEqual((data["run"], data["seconds"], data["warmup"]), ("20261126-120000", 60, 15))
        self.assertEqual([t["identity"] for t in data["title"]], ["TrinityXSX.AetherDrift", "TrinityXSX.NeonVoid"])
        self.assertEqual(data["title"][0]["env"], {"AETHER_AUTO_DEMO": "1"})
        self.assertNotIn("env", data["title"][1])

    def test_request_refuses_unsafe_input(self):
        ok = ("20261126-120000", ["A.B"], 60, 15, {})
        for change in ({1: ["../x"]}, {1: []}, {2: 2}, {3: 9999}, {0: "now"},
                       {4: {"A.B": {"XCL_BENCH_SECONDS": "1"}}}, {4: {"A.B": {"lower": "1"}}},
                       {4: {"A.B": {"X": 'a"b'}}}):
            args = list(ok)
            for i, v in change.items():
                args[i] = v
            with self.assertRaises(bench.BenchError, msg=str(change)):
                bench.request_text(*args)
        self.assertEqual(bench.parse_env_args(["A.B:X=1", "A.B:Y=a=b"]), {"A.B": {"X": "1", "Y": "a=b"}})
        with self.assertRaises(bench.BenchError):
            bench.parse_env_args(["X=1"])

    def test_compare_flags_regressions_beyond_tolerance_only(self):
        base = {"A.B": OK, "C.D": OK}
        results = {
            "A.B": dict(OK, avg_fps=56.0, p99_ms=23.0, stalls_100ms=4),  # within tolerance
            "C.D": dict(OK, avg_fps=50.0, stalls_100ms=9),  # -17 % FPS and +8 stalls
        }
        rows, code = bench.compare(results, base)
        self.assertEqual(code, 1)
        text = "\n".join(rows)
        self.assertIn("| A.B | ok |", text)
        self.assertIn("ok |", rows[2])
        self.assertIn("regression: avg_fps, stalls_100ms", rows[3])
        self.assertIn("50.0 (-17%)", rows[3])

    def test_compare_reports_failed_missing_and_new_titles(self):
        rows, code = bench.compare({"A.B": {"status": "crashed"}, "E.F": OK}, {"A.B": OK, "C.D": OK},
                                   ["A.B", "C.D", "E.F"])
        text = "\n".join(rows)
        self.assertEqual(code, 2)
        self.assertIn("**crashed**", text)
        self.assertIn("| C.D | missing |", text)
        self.assertIn("new (no baseline)", text)
        self.assertEqual(bench.compare({"A.B": OK}, {"A.B": OK})[1], 0)

    def test_a_run_of_some_titles_ignores_the_baselines_other_titles(self):
        rows, code = bench.compare({"A.B": OK}, {"A.B": OK, "C.D": OK, "E.F": OK}, ["A.B"])
        self.assertEqual(code, 0)
        self.assertNotIn("C.D", "\n".join(rows))
        # without a title list, what has a result is the run
        self.assertEqual(bench.compare({"A.B": OK}, {"A.B": OK, "C.D": OK})[1], 0)

    def test_restart_errors_become_bench_errors(self):
        with self.assertRaises(bench.BenchError):
            bench.restart_library("127.0.0.1", "12")
        # nothing listens on this port on a test machine: a connection error, not a traceback
        with self.assertRaises(bench.BenchError):
            bench.restart_library("127.0.0.1", "123456")

    def test_baseline_round_trip_keeps_only_successful_titles(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = Path(tmp) / "run"
            run.mkdir()
            (run / "A.B.json").write_text(json.dumps(dict(OK, identity="A.B")), encoding="utf-8")
            (run / "C.D.json").write_text(json.dumps({"identity": "C.D", "status": "no-frames"}), encoding="utf-8")
            (run / "queue.json").write_text('{"state": "done"}', encoding="utf-8")
            baseline = Path(tmp) / "perf" / "baseline.json"
            self.assertEqual(bench.store_baseline(run, baseline), 0)
            stored = json.loads(baseline.read_text(encoding="utf-8"))["titles"]
            self.assertEqual(list(stored), ["A.B"])
            self.assertEqual(bench.report(run, baseline), 2)  # C.D failed


if __name__ == "__main__":
    unittest.main()
