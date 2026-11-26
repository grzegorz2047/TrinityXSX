# Performance runs on the PS5

`tools/ps5_bench.py` measures how games run on your PS5 and compares the numbers with a stored baseline. Use it to catch a release that makes a game slower, stutter more or use more memory.

## What gets measured

XCL's benchmark mode (`patches/xcl/0029`) does the measuring. It waits for a game's first frame, lets the game run for a warm-up period, then records every presented frame for the measured time. That covers both D3D12.X `PresentX` and native Vulkan titles. Then it writes one JSON file and ends the game:

| Field | Meaning |
| --- | --- |
| `avg_fps` | frames per second over the measured time |
| `p50_ms`, `p95_ms`, `p99_ms`, `max_ms` | frame times |
| `low1_fps` | average frame rate of the slowest 1 % of frames |
| `stalls_100ms` | frames that took over 100 ms (visible hitches) |
| `first_frame_s` | seconds from start to the first frame |
| `peak_commit_mb` | peak committed memory of the process |
| `status` | `ok`, `no-frames`, `crashed`, `not-found`, `unsupported` or `prepare-failed` |

## Running it

1. **Put the games in your library.** The repo's own games are a good fixed set, and so is the XCL Test Title:
   - `samples/aether-drift` (Identity `TrinityXSX.AetherDrift`): run it with `AETHER_AUTO_DEMO=1` so every run plays the same demo.
   - `samples/neon-void` (Identity `TrinityXSX.NeonVoid`).

   A game folder is the sample's `assets/` contents plus its built `.exe`, copied into one of your game folders.
2. **Run it from the PC** (FTP must be on, port 2121):
   ```sh
   python tools/ps5_bench.py run <PS5-IP> --titles TrinityXSX.AetherDrift,TrinityXSX.NeonVoid \
       --env TrinityXSX.AetherDrift:AETHER_AUTO_DEMO=1 --seconds 60 --warmup 15 --pin <PIN on the TV>
   ```
   With `--pin`, the tool restarts the TrinityXSX library through the Web Companion. Without it, start TrinityXSX on the PS5 yourself after the tool has queued the run.
3. **Let it finish.** The library runs one game per start. After each game it starts itself again (through `restart.flag`) until the queue is empty, and then shows the normal menu. The tool waits, downloads the results to `results/<run>/` and prints a table.
4. **The first time, store the results as the baseline:**
   ```sh
   python tools/ps5_bench.py baseline results/<run>
   ```
   Later runs are compared with `perf/baseline.json`. Compare a downloaded run again with:
   ```sh
   python tools/ps5_bench.py compare results/<run>
   ```

The tool exits with **1** when a metric got worse by more than its tolerance:

| Metric | Tolerance |
| --- | --- |
| average FPS | −10 % |
| 1 % low FPS | −15 % |
| p99 frame time | +20 % |
| time to first frame | +25 % |
| peak commit | +15 % |
| frames over 100 ms | +3 |

It exits with **2** when a game failed or has no result. Keep one baseline per console setup (firmware, TrinityXSX settings), because the numbers depend on both.

## On the PS5

- The queue is `/data/homebrew/PPSA83601/bench/request.toml`. Results go to `bench/results/<run>/`, one `<Identity>.json` per game plus `queue.json`.
- A game is taken off the queue **before** it starts. A game that hangs or crashes the console is therefore not started again; it gets `crashed` on the next start.
- To stop a run, delete `bench/request.toml` over FTP.
- A request the library refuses (bad run id, Identity, environment variable name, or time) is renamed to `request.invalid.toml`, and the reason is written to `request.error.txt`.

## Without the library (PC or a single run)

The benchmark mode works with `xcl run` too:
```powershell
$env:XCL_BENCH_SECONDS = "30"; $env:XCL_BENCH_WARMUP = "10"; $env:XCL_BENCH_OUT = "C:\bench\game.json"
xcl run --in-process <game folder>
```

## Notes

- The games and their numbers are your own: do not upload game files with results.
- Opening the Touchpad menu during a run pauses the game and spoils the numbers.
