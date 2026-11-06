#!/usr/bin/env python3
"""Measures the performance targets of docs/VISION.md (docs/PERFORMANCE.md).

Usage: scripts/measure.py <apeiron program> <empty work folder> [--big-gb N] [--files N]
Needs about three times --big-gb of free disk. It uses its own HOME inside the
work folder, so it touches no settings, and it does not remove the work folder.
"""

import argparse
import json
import os
import resource
import signal
import subprocess
import sys
import time
from pathlib import Path


def timed(cmd, env):
    """Runs a command; returns seconds, peak memory in MiB, exit code and output."""
    start = time.monotonic()
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    out = proc.stdout.read()
    _, status, usage = os.wait4(proc.pid, 0)
    proc.returncode = os.waitstatus_to_exitcode(status)
    return time.monotonic() - start, usage.ru_maxrss / 1024, proc.returncode, out


def line(name, target, seconds=None, memory=None, note=""):
    parts = [name, target]
    parts.append("" if seconds is None else f"{seconds:.1f} s")
    parts.append("" if memory is None else f"{memory:.0f} MiB")
    parts.append(note)
    print("| " + " | ".join(parts) + " |", flush=True)


def write_random(path, size, block=1 << 20):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        for _ in range(size // block):
            f.write(os.urandom(block))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("program")
    parser.add_argument("work", type=Path)
    parser.add_argument("--big-gb", type=int, default=10)
    parser.add_argument("--files", type=int, default=100_000)
    args = parser.parse_args()
    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    if any(work.iterdir()):
        sys.exit(f"{work} is not empty")
    home = work / "home"
    home.mkdir()
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / "c"), XDG_STATE_HOME=str(home / "s"))
    env.pop("APEIRON_REPO", None)
    app = [args.program, "--repo", str(work / "repo")]
    subprocess.run([args.program, "init", str(work / "repo")], env=env, check=True, stdout=subprocess.DEVNULL)

    print("| Scenario | Target | Time | Peak memory | Note |")
    print("| --- | --- | --- | --- | --- |")

    # Many small files.
    tree = work / "files"
    for i in range(args.files):
        d = tree / f"{i % 300:03d}"
        if i < 300:
            d.mkdir(parents=True)
        (d / f"{i}.dat").write_bytes(os.urandom(16 * 1024))
    s, m, _, _ = timed(app + ["backup", str(tree), "--ref", "many"], env)
    line(f"First backup of {args.files} files of 16 KiB", "—", s, m)
    s, m, _, _ = timed(app + ["backup", str(tree), "--ref", "many"], env)
    line(f"Backup of the same {args.files} files, unchanged", "under 10 s", s, m)
    s, m, _, _ = timed(app + ["log", "many"], env)
    line("Reading the history of that backup", "under 2 s", s, m)
    s, m, _, _ = timed(app + ["search", "99999*", "--in", "many"], env)
    line(f"Finding a file by name among {args.files}", "under 2 s", s, m)

    # One large file.
    big = work / "big"
    write_random(big / "big.bin", args.big_gb << 30)
    s, m, _, _ = timed(app + ["backup", str(big), "--ref", "big"], env)
    line(f"Adding one file of {args.big_gb} GiB", "under 256 MiB", s, m)
    s, m, _, _ = timed(app + ["backup", str(big), "--ref", "big"], env)
    line(f"Backup of that file, unchanged", "—", s, m, "the content is read again")
    s, m, _, _ = timed(app + ["restore", "big", str(work / "out")], env)
    line(f"Restoring that file", "—", s, m)
    (work / "out" / "big.bin").unlink()

    # The idle background service.
    jobs = home / "jobs.toml"
    jobs.write_text(
        f'[job.a]\nrepo = "{work}/repo"\ndo = "verify"\nevery = "week"\n'
    )
    subprocess.run([args.program, "daemon", "once", "--jobs", str(jobs), "--state-dir", str(home / "d")],
                   env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    proc = subprocess.Popen(
        [args.program, "daemon", "run", "--no-notify", "--jobs", str(jobs), "--state-dir", str(home / "d")],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(10)
    rss = next(int(l.split()[1]) for l in open(f"/proc/{proc.pid}/status") if l.startswith("VmRSS"))
    proc.send_signal(signal.SIGTERM)
    proc.wait()
    line("The background service, idle", "under 50 MiB", None, rss / 1024)

    # An interrupted transfer: stop a push half way and run it again.
    copy = work / "copy"
    subprocess.run(app + ["remote", "add", "disk", str(copy)], env=env, check=True, stdout=subprocess.DEVNULL)
    total = (args.big_gb << 30)
    proc = subprocess.Popen(app + ["push", "disk", "big"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while proc.poll() is None:
        sent = sum(p.stat().st_size for p in copy.rglob("*.pack")) if copy.exists() else 0
        if sent >= total // 2:
            proc.kill()
            break
        time.sleep(0.05)
    proc.wait()
    sent = sum(p.stat().st_size for p in copy.rglob("*.pack")) if copy.exists() else 0
    s, m, code, out = timed(app + ["--json", "push", "disk", "big"], env)
    again = json.loads(out)["bytes"] if code == 0 else -1
    line("Sent again after a push stopped half way", "at most 1 chunk (4 MiB)", s, m,
         f"{sent >> 20} MiB were on the copy; {again >> 20} MiB sent again")


if __name__ == "__main__":
    main()
