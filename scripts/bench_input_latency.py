#!/usr/bin/env python3
"""Input-latency harness for camouflage-tui.

Pipes fake-agent events into camouflage-tui under a pty. Periodically injects
single character keystrokes and measures the wall-clock interval between
write(master, key) and the corresponding character appearing on the master's
read side (which is what the user's eye would see).

Reports min / mean / p50 / p95 / p99 / max in milliseconds across N samples.

Usage:
    python3 scripts/bench_input_latency.py [--samples 200] [--interval 0.05]
"""
from __future__ import annotations

import argparse
import os
import pty
import fcntl
import termios
import struct
import select
import signal
import statistics
import subprocess
import sys
import time


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument('--samples', type=int, default=200)
    p.add_argument('--interval', type=float, default=0.05,
                   help='seconds between keystrokes')
    p.add_argument('--tokens', type=int, default=200_000,
                   help='tokens fake-agent emits to keep stream busy')
    p.add_argument('--tools', type=int, default=500)
    p.add_argument('--rows', type=int, default=30)
    p.add_argument('--cols', type=int, default=120)
    args = p.parse_args()

    # Repo root assumed to be the working dir.
    tui_bin = os.path.join('target', 'release', 'camouflage-tui')
    fake_bin = os.path.join('target', 'release', 'fake-agent')
    if not (os.path.exists(tui_bin) and os.path.exists(fake_bin)):
        print('missing release binaries. Run `cargo build --release` first.',
              file=sys.stderr)
        return 2

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ,
                struct.pack('HHHH', args.rows, args.cols, 0, 0))

    def setup_child() -> None:
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    cmd = (
        f'{fake_bin} --tokens {args.tokens} --tools {args.tools} --fast | '
        f'{tui_bin} --stdin-events --fps 60 2>/tmp/bench_input_latency_stderr.log'
    )
    proc = subprocess.Popen(
        ['bash', '-c', cmd],
        stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=setup_child,
    )
    os.close(slave)

    # Wait until the TUI has drawn something — gives us the initial alt-screen
    # output before we start injecting keystrokes.
    deadline = time.time() + 3
    primed = False
    drained = b''
    while time.time() < deadline and not primed:
        r, _, _ = select.select([master], [], [], 0.1)
        if r:
            try:
                drained += os.read(master, 32768)
            except OSError:
                break
        if b'Camouflage' in drained or b'\x1b[?1049h' in drained:
            primed = True

    if not primed:
        proc.send_signal(signal.SIGINT)
        proc.wait(timeout=2)
        print('TUI did not prime within 3s', file=sys.stderr)
        return 3

    latencies_ms: list[float] = []
    samples = args.samples
    # The TUI redraws its input box on the next 16ms tick after a keystroke,
    # and ratatui's diff render writes the changed cells (including our char)
    # interspersed with whatever transcript bytes the stream produced. We
    # accumulate bytes across reads inside the per-sample window and search
    # the accumulated buffer for the keystroke.
    #
    # Use a unique signal sequence not present in the transcript: send the
    # ASCII record-separator (0x1e) — wait, ratatui won't render that. Better:
    # send a unique printable char and rely on the fact that the input row is
    # the only place the renderer puts arbitrary user-typed chars. The risk
    # is the transcript already contains 'a' bytes. To disambiguate we look
    # for the character followed by the cursor-hide sequence \x1b[?25l, which
    # ratatui emits at the END of every full frame draw — i.e. the char must
    # be in the latest frame, not stale transcript bytes.
    # Use punctuation that fake-agent never emits in its transcript content
    # (which is limited to "tok", "bash npm test --silent", "ok\n", etc.).
    # This avoids false positives where the streamed transcript already
    # contains our keystroke byte.
    chars = b'~^`!@#$%&+=?'
    for i in range(samples):
        ch = chars[i % len(chars):i % len(chars) + 1]
        # Drain any pending output BEFORE injecting, so subsequent reads only
        # contain new bytes.
        while True:
            r, _, _ = select.select([master], [], [], 0)
            if not r:
                break
            try:
                _ = os.read(master, 65536)
            except OSError:
                break
        start = time.perf_counter()
        os.write(master, ch)
        deadline = start + 0.5
        seen_at: float | None = None
        accum = b''
        # Look for the char's CUP-rewrite signature: ratatui places the
        # character on the input row inside the box. We just look for the
        # raw byte in the accumulating buffer; on a 60 FPS tick the first
        # frame that includes our char arrives within ~16ms.
        while time.perf_counter() < deadline and seen_at is None:
            r, _, _ = select.select([master], [], [], 0.002)
            if r:
                try:
                    data = os.read(master, 16384)
                except OSError:
                    break
                if not data:
                    break
                accum += data
                if ch in accum:
                    seen_at = time.perf_counter()
        if seen_at is not None:
            latencies_ms.append((seen_at - start) * 1000.0)
        # Erase our character so it doesn't pile up in the input buffer.
        try:
            os.write(master, b'\x7f')
        except OSError:
            break
        time.sleep(args.interval)

    # Clean shutdown.
    try:
        os.write(master, b'q')
    except OSError:
        pass
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        proc.kill()

    if not latencies_ms:
        print('no successful samples', file=sys.stderr)
        # Print any captured stderr to help debug
        try:
            with open('/tmp/bench_input_latency_stderr.log', 'r') as f:
                err = f.read().strip()
                if err:
                    print('TUI stderr:', file=sys.stderr)
                    print(err, file=sys.stderr)
        except OSError:
            pass
        return 4

    latencies_ms.sort()
    def pct(p: float) -> float:
        idx = min(len(latencies_ms) - 1, int(round((len(latencies_ms) - 1) * p)))
        return latencies_ms[idx]

    print(f'samples       {len(latencies_ms)} of {samples}')
    print(f'min           {latencies_ms[0]:.2f} ms')
    print(f'mean          {statistics.fmean(latencies_ms):.2f} ms')
    print(f'p50           {pct(0.50):.2f} ms')
    print(f'p95           {pct(0.95):.2f} ms')
    print(f'p99           {pct(0.99):.2f} ms')
    print(f'max           {latencies_ms[-1]:.2f} ms')
    print(f'target p95    < 25.00 ms')
    print('result        PASS' if pct(0.95) < 25.0 else 'result        FAIL')
    return 0 if pct(0.95) < 25.0 else 1


if __name__ == '__main__':
    sys.exit(main())
