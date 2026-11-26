#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Automatic TrinityXSX performance runs on a PS5, compared with a baseline.

The library on the PS5 runs a queue of games from <storage root>/bench/request.toml, each for a
measured time (XCL_BENCH_SECONDS), and writes one JSON per game to bench/results/<run>/. See
docs/PERFORMANCE.md.

  python tools/ps5_bench.py run <PS5-IP> --titles TrinityXSX.AetherDrift,TrinityXSX.NeonVoid \\
      [--seconds 60] [--warmup 15] [--env TrinityXSX.AetherDrift:AETHER_AUTO_DEMO=1] [--pin 123456]
  python tools/ps5_bench.py compare results/<run> [--baseline perf/baseline.json]
  python tools/ps5_bench.py baseline results/<run>      store a run as the new baseline

`run` uploads the request over FTP (port 2121), restarts the library through the Web Companion when
--pin is given (otherwise start TrinityXSX by hand), waits for the queue, downloads the results to
results/<run>/ and compares them. It exits 1 on a regression, 2 on a failed or missing title.
"""
from __future__ import annotations

import argparse
import ftplib
import http.client
import io
import json
import re
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = "/data/homebrew/PPSA83601"
BENCH = f"{BASE}/bench"
DEFAULT_BASELINE = ROOT / "perf" / "baseline.json"

IDENTITY = re.compile(r"[A-Za-z0-9][A-Za-z0-9.\-]{1,99}")
ENV_NAME = re.compile(r"[A-Z][A-Z0-9_]{0,63}")
RUN_ID = re.compile(r"\d{8}-\d{6}")

# A metric regresses when it is worse than the baseline by more than this fraction.
TOLERANCE = {"avg_fps": 0.10, "low1_fps": 0.15, "p99_ms": 0.20, "first_frame_s": 0.25, "peak_commit_mb": 0.15}
HIGHER_IS_BETTER = {"avg_fps", "low1_fps"}
# a few more >100 ms frames than the baseline are noise; more than this is a regression
STALL_SLACK = 3


class BenchError(ValueError):
    pass


# ---- request ---------------------------------------------------------------------------------

def request_text(run_id: str, titles: list[str], seconds: int, warmup: int, env: dict[str, dict[str, str]]) -> str:
    if not RUN_ID.fullmatch(run_id):
        raise BenchError(f"bad run id {run_id!r}")
    if not 5 <= seconds <= 1800 or not 0 <= warmup <= 600:
        raise BenchError("seconds must be 5..1800 and warmup 0..600")
    if not titles:
        raise BenchError("no titles")
    lines = [
        "# TrinityXSX benchmark queue (tools/ps5_bench.py). The library runs and removes one title per start.",
        f'run = "{run_id}"',
        f"seconds = {seconds}",
        f"warmup = {warmup}",
    ]
    for identity in titles:
        if not IDENTITY.fullmatch(identity):
            raise BenchError(f"bad Identity Name {identity!r}")
        lines += ["", "[[title]]", f'identity = "{identity}"']
        pairs = env.get(identity, {})
        for name, value in pairs.items():
            if not ENV_NAME.fullmatch(name) or name.startswith("XCL_BENCH"):
                raise BenchError(f"environment variable {name!r} is not allowed")
            if any(c in value for c in '"\\\n\r'):
                raise BenchError(f"value of {name} contains a quote, backslash or newline")
        if pairs:
            lines.append("env = { " + ", ".join(f'{k} = "{v}"' for k, v in pairs.items()) + " }")
    return "\n".join(lines) + "\n"


def parse_env_args(items: list[str]) -> dict[str, dict[str, str]]:
    out: dict[str, dict[str, str]] = {}
    for item in items:
        m = re.fullmatch(r"([^:]+):([^=]+)=(.*)", item)
        if not m:
            raise BenchError(f"--env expects Identity:NAME=value, got {item!r}")
        out.setdefault(m.group(1), {})[m.group(2)] = m.group(3)
    return out


# ---- results and comparison ------------------------------------------------------------------

def load_results(folder: Path) -> dict[str, dict]:
    results = {}
    for path in sorted(folder.glob("*.json")):
        if path.name == "queue.json":
            continue
        data = json.loads(path.read_text(encoding="utf-8"))
        results[data.get("identity", path.stem)] = data
    return results


def compare(results: dict[str, dict], baseline: dict[str, dict], titles: list[str] | None = None) -> tuple[list[str], int]:
    """Markdown report lines and an exit code: 0 ok, 1 regression, 2 failed or missing title.

    Only the titles of this run count (`titles`, else those with a result): a run of a few games is
    not failed by the baseline's other games.
    """
    code = 0
    rows = ["| Title | Status | avg FPS | 1% low FPS | p99 ms | >100 ms frames | first frame s | peak commit MiB | Verdict |",
            "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for identity in sorted(set(titles) if titles is not None else set(results)):
        r, b = results.get(identity), baseline.get(identity)
        if r is None:
            rows.append(f"| {identity} | missing | | | | | | | **no result** |")
            code = max(code, 2)
            continue
        if r.get("status") != "ok":
            rows.append(f"| {identity} | {r.get('status')} | | | | | | | **{r.get('status')}** |")
            code = max(code, 2)
            continue
        cells, worse = [], []
        for metric in ("avg_fps", "low1_fps", "p99_ms", "stalls_100ms", "first_frame_s", "peak_commit_mb"):
            value = r.get(metric)
            base = b.get(metric) if b and b.get("status") == "ok" else None
            cells.append(cell(value, base))
            if value is None or base is None:
                continue
            if metric == "stalls_100ms":
                if value > base + STALL_SLACK:
                    worse.append(metric)
            elif metric in HIGHER_IS_BETTER:
                if value < base * (1 - TOLERANCE[metric]):
                    worse.append(metric)
            elif base > 0 and value > base * (1 + TOLERANCE[metric]):
                worse.append(metric)
        verdict = "new (no baseline)" if b is None else ("**regression: " + ", ".join(worse) + "**" if worse else "ok")
        if worse:
            code = max(code, 1)
        rows.append(f"| {identity} | ok | " + " | ".join(cells) + f" | {verdict} |")
    return rows, code


def cell(value, base) -> str:
    if value is None:
        return ""
    text = f"{value:.1f}" if isinstance(value, float) else str(value)
    if base is None or base == 0:
        return text
    return f"{text} ({(value - base) / base * 100:+.0f}%)"


# ---- PS5 transport ---------------------------------------------------------------------------

def ftp_connect(host: str, port: int) -> ftplib.FTP:
    ftp = ftplib.FTP()
    ftp.connect(host, port, timeout=20)
    ftp.login()
    return ftp


def ftp_mkdirs(ftp: ftplib.FTP, path: str) -> None:
    cur = ""
    for part in path.strip("/").split("/"):
        cur += "/" + part
        try:
            ftp.mkd(cur)
        except ftplib.error_perm:
            pass


def ftp_read(ftp: ftplib.FTP, path: str) -> bytes | None:
    buf = io.BytesIO()
    try:
        ftp.retrbinary(f"RETR {path}", buf.write)
    except ftplib.error_perm:
        return None
    return buf.getvalue()


def restart_library(host: str, pin: str) -> None:
    """Restart the library through the Web Companion (PIN -> session cookie -> POST /restart)."""
    if not re.fullmatch(r"\d{6}", pin):
        raise BenchError("--pin must be the 6-digit PIN shown on the TV")
    try:
        _restart_library(host, pin)
    except (OSError, http.client.HTTPException) as e:
        raise BenchError(f"Web Companion not reachable on {host}:8360 ({e})") from e


def _restart_library(host: str, pin: str) -> None:
    conn = http.client.HTTPConnection(host, 8360, timeout=10)
    conn.request("GET", f"/?pin={pin}")
    resp = conn.getresponse()
    resp.read()
    cookie = resp.getheader("Set-Cookie", "").split(";", 1)[0]
    if resp.status != 303 or not cookie.startswith("txsx_session="):
        raise BenchError(f"Web Companion refused the PIN (HTTP {resp.status})")
    conn = http.client.HTTPConnection(host, 8360, timeout=10)
    conn.request("POST", "/restart", body=b"", headers={"Cookie": cookie, "Origin": f"http://{host}:8360",
                                                        "Content-Length": "0"})
    resp = conn.getresponse()
    resp.read()
    if resp.status >= 400:
        raise BenchError(f"restart failed (HTTP {resp.status})")


def run(args) -> int:
    run_id = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    titles = [t.strip() for t in args.titles.split(",") if t.strip()]
    text = request_text(run_id, titles, args.seconds, args.warmup, parse_env_args(args.env or []))
    ftp = ftp_connect(args.host, args.port)
    ftp_mkdirs(ftp, f"{BENCH}/results/{run_id}")
    ftp.storbinary(f"STOR {BENCH}/request.toml", io.BytesIO(text.encode("utf-8")))
    ftp.quit()
    print(f"queued run {run_id}: {', '.join(titles)}")
    if args.pin:
        try:
            restart_library(args.host, args.pin)
        except BenchError:
            # do not leave a queue behind that would start on its own later, unattended
            try:
                ftp = ftp_connect(args.host, args.port)
                ftp.delete(f"{BENCH}/request.toml")
                ftp.quit()
                print("restart failed: the queued run was removed from the PS5", file=sys.stderr)
            except (OSError, ftplib.Error):
                print(f"restart failed and {BENCH}/request.toml could not be removed: delete it over FTP",
                      file=sys.stderr)
            raise
        print("library restarting")
    else:
        print("start (or restart) TrinityXSX on the PS5 to begin")
    # every title: load time + warmup + measured time; a generous bound, then give up
    deadline = time.monotonic() + len(titles) * (args.seconds + args.warmup + 300) + 300
    remote = f"{BENCH}/results/{run_id}"
    local = Path(args.out) / run_id
    local.mkdir(parents=True, exist_ok=True)
    while True:
        try:
            ftp = ftp_connect(args.host, args.port)
            done = ftp_read(ftp, f"{remote}/queue.json")
            if done is not None and json.loads(done).get("state") == "done":
                for identity in titles:
                    data = ftp_read(ftp, f"{remote}/{identity}.json")
                    if data is not None:
                        (local / f"{identity}.json").write_bytes(data)
                (local / "queue.json").write_bytes(done)
                ftp.quit()
                break
            ftp.quit()
        except (OSError, ftplib.Error) as e:
            print(f"(waiting: {e})")
        if time.monotonic() > deadline:
            print("timed out waiting for the benchmark queue", file=sys.stderr)
            return 2
        time.sleep(15)
    print(f"results in {local}")
    return report(local, Path(args.baseline), titles)


def report(folder: Path, baseline_path: Path, titles: list[str] | None = None) -> int:
    baseline = json.loads(baseline_path.read_text(encoding="utf-8")).get("titles", {}) if baseline_path.is_file() else {}
    rows, code = compare(load_results(folder), baseline, titles)
    print("\n".join(rows))
    return code


def store_baseline(folder: Path, baseline_path: Path) -> int:
    results = {k: v for k, v in load_results(folder).items() if v.get("status") == "ok"}
    if not results:
        print("no successful results to store", file=sys.stderr)
        return 2
    data = json.loads(baseline_path.read_text(encoding="utf-8")) if baseline_path.is_file() else {}
    data.setdefault("titles", {}).update(results)
    data["note"] = "PS5 benchmark baseline (tools/ps5_bench.py baseline <results>); one entry per Identity Name."
    baseline_path.parent.mkdir(parents=True, exist_ok=True)
    baseline_path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"stored {len(results)} title(s) in {baseline_path}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("host")
    r.add_argument("--port", type=int, default=2121)
    r.add_argument("--titles", required=True, help="comma-separated Identity Names")
    r.add_argument("--seconds", type=int, default=60)
    r.add_argument("--warmup", type=int, default=15)
    r.add_argument("--env", action="append", help="Identity:NAME=value for one title")
    r.add_argument("--pin", help="Web Companion PIN: restart the library automatically")
    r.add_argument("--out", default="results")
    r.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    c = sub.add_parser("compare")
    c.add_argument("folder")
    c.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    c.add_argument("--titles", help="comma-separated Identity Names the run had (a missing one fails)")
    b = sub.add_parser("baseline")
    b.add_argument("folder")
    b.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    args = ap.parse_args()
    try:
        if args.cmd == "run":
            return run(args)
        if args.cmd == "compare":
            titles = [t.strip() for t in args.titles.split(",") if t.strip()] if args.titles else None
            return report(Path(args.folder), Path(args.baseline), titles)
        return store_baseline(Path(args.folder), Path(args.baseline))
    except BenchError as e:
        print(f"ps5_bench: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
