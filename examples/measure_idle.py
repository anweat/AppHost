#!/usr/bin/env python3
"""Measure an isolated release daemon, with no Apps, UI or log subscribers."""
import argparse
import json
import pathlib
import platform
import socket
import statistics
import subprocess
import tempfile
import time


def process_table():
    rows = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,rss=,time="], text=True)
    table = {}
    for line in rows.splitlines():
        pid, parent, rss, cpu = line.split()
        pieces = cpu.replace("-", ":").split(":")
        seconds = sum(float(part) * factor for part, factor in zip(reversed(pieces), [1, 60, 3600, 86400]))
        table[int(pid)] = (int(parent), int(rss) * 1024, seconds)
    return table


def owned(table, root):
    found = {root}
    while True:
        new = {pid for pid, row in table.items() if row[0] in found} - found
        if not new:
            return found
        found.update(new)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/apphost"))
    parser.add_argument("--seconds", type=int, default=60)
    args = parser.parse_args()
    if platform.system() != "Darwin" or not 1 <= args.seconds <= 600:
        parser.error("Requires macOS and 1..600 seconds")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="ah-idle-", dir="/tmp") as directory:
        data = pathlib.Path(directory) / "data"
        cli = [str(binary), "--data-dir", str(data)]
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        subprocess.run(cli + ["gateway", "set-port", str(port)], check=True, capture_output=True)
        daemon = subprocess.Popen(cli + ["daemon", "serve"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 10
            while subprocess.run(cli + ["daemon", "ping"], capture_output=True).returncode:
                if daemon.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Daemon did not become ready")
                time.sleep(0.05)
            time.sleep(5)
            status = json.loads(subprocess.check_output(cli + ["status"]))
            if status["instances"] or not status["gateway"]["available"]:
                raise RuntimeError("Expected an empty daemon with an available gateway")
            baseline = process_table()
            population = owned(baseline, daemon.pid)
            start_cpu = sum(baseline[pid][2] for pid in population)
            start = time.monotonic()
            samples = []
            while True:
                now = time.monotonic()
                table = process_table()
                current = owned(table, daemon.pid)
                if daemon.poll() is not None or current != population or not population <= table.keys():
                    raise RuntimeError("Background process population changed; measurement is incomplete")
                samples.append(sum(table[pid][1] for pid in population))
                if now - start >= args.seconds:
                    break
                time.sleep(min(1, args.seconds - (now - start)))
            elapsed = time.monotonic() - start
            cpu = sum(table[pid][2] for pid in population) - start_cpu
            result = {
                "platform": platform.platform(), "machine": platform.machine(),
                "hardware": subprocess.check_output(["/usr/sbin/sysctl", "-n", "hw.model"], text=True).strip(),
                "version": subprocess.check_output([str(binary), "--version"], text=True).strip(),
                "binary": str(binary), "duration_seconds": elapsed, "warmup_seconds": 5,
                "background_processes": len(population), "rss_peak_bytes": max(samples),
                "rss_mean_bytes": statistics.mean(samples), "cpu_seconds": cpu,
                "cpu_mean_percent_one_core": cpu / elapsed * 100,
                "cpu_time_resolution_seconds": 0.01,
                "network_bytes_measured": False,
                "scope": "isolated daemon and all descendants; no Apps, UI or subscriptions",
            }
            print(json.dumps(result, ensure_ascii=False, indent=2))
        finally:
            if daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()


if __name__ == "__main__":
    main()
