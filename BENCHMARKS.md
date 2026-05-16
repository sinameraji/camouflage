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

## Baseline run

- Hardware: Darwin 25.4.0 / Apple Silicon (developer laptop)
- Build: `cargo run --release` (LTO=thin, codegen-units=1)
- Command: `--events 100000 --tools 1000 --tokens 50000`

| Metric                | Value     | Target  | Status |
|-----------------------|-----------|---------|--------|
| `replay_ms`           | 72        | < 5000  | ✅      |
| `p95_frame_ms`        | 0.009     | < 16    | ✅      |
| `rss_mb`              | 187       | < 200   | ✅      |
| `event_write_per_sec` | 591,715   | —       | —      |
| `range_read_ms`       | 0         | —       | —      |

Input latency under streaming load is not measured by the offline bench
harness (no terminal in the loop). It is covered by manual TUI testing.

## Known caveats

- The current bench loads all generated events into memory (`Vec<Event>`) for the persistence-throughput phase. This dominates RSS at 100k events; the renderer itself is bounded at 2000 rows. A subsequent revision should stream events into the store rather than materialising the vector.
- The `model.dirty()` tick model coalesces frames; under sustained 50k tok/s bursts, render frequency is governed by the ticker, not the token rate.

## Next benchmarks (v0.2+)

- 5-hour soak via `fake-agent --duration 18000 --fast | camouflage-tui --stdin-events`, sampling RSS every 60s.
- Terminal-in-the-loop p95 input latency using a pty-driven harness.
- Range-read latency at session sizes > 1M events.
