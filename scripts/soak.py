#!/usr/bin/env python3
"""Soak test for camouflage-tui.

Spawns `fake-agent ... | camouflage-tui --stdin-events` under a pty,
samples RSS of the TUI process every N seconds, and reports peak / final.
Fails (exit 1) if RSS exceeds the cap at any sample.

Usage:
    python3 scripts/soak.py --duration 18000     # full 5h soak
    python3 scripts/soak.py --duration 600       # 10min validation
"""
from __future__ import annotations

import argparse
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time


def find_tui_pid(parent_pid: int) -> int | None:
    """Find the camouflage-tui pid in the descendants of parent_pid."""
    try:
        out = subprocess.check_output(
            ['pgrep', '-P', str(parent_pid)], text=True
        ).strip().splitlines()
    except subprocess.CalledProcessError:
        return None
    for line in out:
        pid = int(line.strip())
        try:
            with open(f'/proc/{pid}/comm') as f:
                if 'camouflage-tui' in f.read():
                    return pid
        except FileNotFoundError:
            pass
        # macOS fallback: ps
        try:
            cmd = subprocess.check_output(
                ['ps', '-p', str(pid), '-o', 'comm='], text=True
            ).strip()
            if 'camouflage-tui' in cmd:
                return pid
        except subprocess.CalledProcessError:
            pass
    return None


def rss_mb(pid: int) -> int:
    """Read RSS in MB for `pid` (macOS via ps)."""
    try:
        out = subprocess.check_output(
            ['ps', '-o', 'rss=', '-p', str(pid)], text=True
        )
        kb = int(out.strip())
        return kb // 1024
    except (subprocess.CalledProcessError, ValueError):
        return -1


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument('--duration', type=int, default=600,
                   help='soak duration in seconds')
    p.add_argument('--sample-interval', type=int, default=60,
                   help='RSS sample interval in seconds')
    p.add_argument('--cap-mb', type=int, default=200,
                   help='fail if RSS exceeds this many MB')
    p.add_argument('--tokens', type=int, default=10_000_000,
                   help='fake-agent tokens (sized so the stream lasts the duration)')
    p.add_argument('--tools', type=int, default=200_000)
    p.add_argument('--log', default='/tmp/camouflage_soak.log')
    args = p.parse_args()

    tui_bin = os.path.join('target', 'release', 'camouflage-tui')
    fake_bin = os.path.join('target', 'release', 'fake-agent')
    if not (os.path.exists(tui_bin) and os.path.exists(fake_bin)):
        print('missing release binaries.', file=sys.stderr)
        return 2

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))

    def setup_child() -> None:
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    cmd = (
        f'{fake_bin} --tokens {args.tokens} --tools {args.tools} '
        f'--duration {args.duration} | '
        f'{tui_bin} --stdin-events --fps 30 2>/tmp/camouflage_soak_tui_stderr.log'
    )
    proc = subprocess.Popen(
        ['bash', '-c', cmd],
        stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=setup_child,
    )
    os.close(slave)

    # Give the TUI a couple seconds to come up and find its pid.
    time.sleep(2)
    tui_pid = find_tui_pid(proc.pid)
    if tui_pid is None:
        print('could not locate camouflage-tui pid', file=sys.stderr)
        proc.kill()
        return 3

    log = open(args.log, 'w')
    print(f'soak start  pid={tui_pid}  duration={args.duration}s  '
          f'cap={args.cap_mb}MB  log={args.log}')
    log.write(f'# soak pid={tui_pid} duration={args.duration} cap_mb={args.cap_mb}\n')
    log.write('elapsed_s,rss_mb\n')
    log.flush()

    start = time.time()
    peak = 0
    samples: list[tuple[int, int]] = []
    next_sample = start
    end = start + args.duration
    breached = False
    drain_buf = bytearray()

    while time.time() < end and proc.poll() is None:
        # Drain pty output so it doesn't block the slave.
        r, _, _ = select.select([master], [], [], 0.5)
        if r:
            try:
                drain_buf += os.read(master, 65536)
                if len(drain_buf) > 1_000_000:
                    drain_buf = drain_buf[-65536:]
            except OSError:
                break

        now = time.time()
        if now >= next_sample:
            elapsed = int(now - start)
            r_mb = rss_mb(tui_pid)
            samples.append((elapsed, r_mb))
            peak = max(peak, r_mb)
            log.write(f'{elapsed},{r_mb}\n')
            log.flush()
            print(f't+{elapsed:6d}s  rss={r_mb}MB  peak={peak}MB')
            if r_mb > args.cap_mb:
                breached = True
                print(f'!! RSS {r_mb}MB exceeded cap {args.cap_mb}MB', file=sys.stderr)
            next_sample = now + args.sample_interval

    # Cleanup
    try:
        os.write(master, b'q')
    except OSError:
        pass
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()

    elapsed = int(time.time() - start)
    final = rss_mb(tui_pid) if tui_pid else -1
    print(f'soak end    elapsed={elapsed}s  peak_rss={peak}MB  '
          f'samples={len(samples)}  result={"FAIL" if breached else "PASS"}')
    log.write(f'# elapsed={elapsed} peak_rss={peak} samples={len(samples)} '
              f'result={"FAIL" if breached else "PASS"}\n')
    log.close()

    return 1 if breached else 0


if __name__ == '__main__':
    sys.exit(main())
