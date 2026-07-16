#!/usr/bin/env python3
"""Fail when the macOS product shell consumes unbounded RSS or CPU while idle."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--duration-seconds", type=int, default=180)
    parser.add_argument("--warmup-seconds", type=int, default=30)
    parser.add_argument("--sample-seconds", type=int, default=5)
    parser.add_argument("--rss-limit-mib", type=float, default=512.0)
    parser.add_argument("--rss-growth-limit-mib", type=float, default=128.0)
    parser.add_argument("--cpu-limit-percent", type=float, default=25.0)
    return parser.parse_args()


def read_process_sample(pid: int) -> tuple[int, float] | None:
    result = subprocess.run(
        ["ps", "-p", str(pid), "-o", "rss=,%cpu="],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return None
    fields = result.stdout.split()
    if len(fields) != 2:
        raise RuntimeError(f"unexpected ps output: {result.stdout!r}")
    return int(fields[0]), float(fields[1])


def stop_process(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def main() -> int:
    args = parse_args()
    if sys.platform != "darwin":
        raise SystemExit("macos-idle-soak requires Darwin")
    if args.duration_seconds < 180:
        raise SystemExit("duration must be at least 180 seconds")
    if args.warmup_seconds <= 0 or args.warmup_seconds >= args.duration_seconds:
        raise SystemExit("warmup must be positive and shorter than duration")
    if args.sample_seconds <= 0:
        raise SystemExit("sample interval must be positive")
    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"binary is not executable: {binary}")

    with tempfile.TemporaryDirectory(prefix="rutile-idle-soak-") as home:
        env = os.environ.copy()
        env["HOME"] = home
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            process = subprocess.Popen(
                [str(binary)],
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=stdout,
                stderr=stderr,
            )
            started = time.monotonic()
            samples: list[tuple[float, int, float]] = []
            failure: str | None = None
            try:
                while True:
                    elapsed = time.monotonic() - started
                    if elapsed >= args.duration_seconds:
                        break
                    sample = read_process_sample(process.pid)
                    if sample is None:
                        failure = f"Rutile exited during idle soak with status {process.poll()}"
                        break
                    rss_kib, cpu_percent = sample
                    if elapsed >= args.warmup_seconds:
                        samples.append((elapsed, rss_kib, cpu_percent))
                    time.sleep(min(args.sample_seconds, args.duration_seconds - elapsed))
            finally:
                stop_process(process)

            if not samples:
                failure = failure or "idle soak collected no post-warmup samples"
                baseline_kib = peak_kib = 0
                final_cpu = 0.0
            else:
                baseline_kib = samples[0][1]
                peak_kib = max(sample[1] for sample in samples)
                final_cpu = samples[-1][2]
                if peak_kib > args.rss_limit_mib * 1024:
                    failure = (
                        f"peak RSS {peak_kib / 1024:.1f} MiB exceeded "
                        f"{args.rss_limit_mib:.1f} MiB"
                    )
                elif peak_kib - baseline_kib > args.rss_growth_limit_mib * 1024:
                    failure = (
                        f"RSS grew {(peak_kib - baseline_kib) / 1024:.1f} MiB after warmup; "
                        f"limit is {args.rss_growth_limit_mib:.1f} MiB"
                    )
                elif final_cpu > args.cpu_limit_percent:
                    failure = (
                        f"idle CPU {final_cpu:.1f}% exceeded "
                        f"{args.cpu_limit_percent:.1f}% (idle redraw loop suspected)"
                    )

            result = {
                "duration_seconds": args.duration_seconds,
                "warmup_seconds": args.warmup_seconds,
                "samples": len(samples),
                "baseline_rss_mib": round(baseline_kib / 1024, 1),
                "peak_rss_mib": round(peak_kib / 1024, 1),
                "final_cpu_percent": final_cpu,
                "status": "failed" if failure else "passed",
            }
            print(json.dumps(result, sort_keys=True))
            if failure:
                stderr.seek(0)
                captured = stderr.read(16 * 1024).decode("utf-8", errors="replace")
                print(f"macos-idle-soak: {failure}", file=sys.stderr)
                if captured:
                    print(captured, file=sys.stderr)
                return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
