# Benchmarks

## Target metrics (v0.1)

| Metric                                   | Target  |
|------------------------------------------|---------|
| RSS after 5h simulated session           | < 200MB |
| 100k-event replay                        | < 5s    |
| p95 render frame time during streaming   | < 16ms  |
| p95 input latency during streaming       | < 25ms  |

## How to run

```bash
cargo run --release -p camouflage-bench -- \
  --events 100000 --tools 1000 --tokens 50000
```

Output is a single JSON object on stdout.

## Baseline run (streaming bench)

- Hardware: Darwin 25.4.0 / Apple Silicon (developer laptop)
- Build: `cargo run --release` (LTO=thin, codegen-units=1)
- Command: `--events 100000 --tools 1000 --tokens 50000`

| Metric                  | Value     | Target  | Status |
|-------------------------|-----------|---------|--------|
| `replay_ms`             | 84        | < 5000  | ✅      |
| `p95_frame_ms`          | 0.732     | < 16    | ✅      |
| `rss_after_write_mb`    | 34        | < 200   | ✅      |
| `rss_after_replay_mb`   | 37        | < 200   | ✅      |
| `event_write_per_sec`   | 240,963   | —       | —      |
| `range_read_ms`         | 0         | —       | —      |

The bench now streams synthesized events directly into the store in
batches of 256 and never materialises the full population. RSS at the
end reflects the renderer's bounded model + SQLite cache, not a
`Vec<Event>` blowup. (Previously: 187 MB.)

## Input latency (pty harness)

- Harness: `scripts/bench_input_latency.py`
- Stream: `fake-agent --tokens 200000 --tools 500 --fast`
- 100 samples, 40 ms apart, 60 fps target

| Metric  | Value      | Target  | Status |
|---------|------------|---------|--------|
| min     | 0.02 ms    | —       | —      |
| mean    | 9.94 ms    | —       | —      |
| p50     | 1.35 ms    | —       | —      |
| **p95** | **23.99 ms** | < 25 ms | ✅    |
| p99     | 25.78 ms   | —       | —      |
| max     | 26.53 ms   | —       | —      |

The min is unrealistically low — likely a race where stale frame bytes
matched the keystroke search before the actual draw arrived. Mean and
p95 are the trustworthy numbers. The harness disambiguates by injecting
characters that fake-agent's transcript never contains (`~^\`!@#$%&+=?`).

## Known caveats

- The `model.dirty()` tick model coalesces frames; under sustained 50k tok/s bursts, render frequency is governed by the ticker, not the token rate.
- 5-hour soak run is documented as planned but not yet executed (PROGRESS.md #18).

## Next benchmarks (v0.2+)

- 5-hour soak via `fake-agent --duration 18000 --fast | camouflage-tui --stdin-events`, sampling RSS every 60s.
- Terminal-in-the-loop p95 input latency using a pty-driven harness.
- Range-read latency at session sizes > 1M events.
