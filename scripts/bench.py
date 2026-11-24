#!/usr/bin/env python3
"""Measures Hostlane against the performance targets in docs/requirements.md section 6.

Usage: scripts/bench.py [path/to/hostlane]   (default: target/release/hostlane)

Uses the throwaway sshd from testbed.py, so the server is this machine: numbers show
Hostlane's own overhead, not network latency.
"""

import os
import socket
import statistics
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from testbed import ROOT, Testbed

HOSTS = 50
TUNNELS = 100
EXEC_RUNS = 100
CLI_RUNS = 30
TRANSFER_MB = 256


def rss_mb(pid):
    out = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True)
    return int(out.strip()) / 1024


def timed(fn, runs):
    samples = []
    for _ in range(runs):
        start = time.perf_counter()
        fn()
        samples.append((time.perf_counter() - start) * 1000)
    samples.sort()
    return statistics.median(samples), samples[int(len(samples) * 0.95) - 1]


def blob_server():
    """A TCP server that sends TRANSFER_MB of data once the client sends a byte."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(16)
    block = os.urandom(1 << 20)

    def send(conn):
        with conn:
            try:
                if conn.recv(1):
                    for _ in range(TRANSFER_MB):
                        conn.sendall(block)
            except OSError:
                pass

    def serve():
        while True:
            conn, _ = listener.accept()
            threading.Thread(target=send, args=(conn,), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()
    return listener.getsockname()[1]


def main():
    bed = Testbed(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release/hostlane")
    rows = []
    try:
        bed.start_sshd()
        aliases = ["test"] + [f"h{i:02}" for i in range(1, HOSTS)]
        bed.write_ssh_config(aliases)
        bed.trust_in_known_hosts()
        bed.run("daemon", "start")
        pid = bed.rpc("daemon.status")["pid"]
        time.sleep(1)
        rows.append(("Daemon memory, idle (RSS)", f"{rss_mb(pid):.1f} MB", "< 30 MB"))

        bed.rpc("hosts.connect", {"host": "test"})
        exec_true = lambda: bed.rpc("exec.run", {"host": "test", "command": "true"})
        exec_true()
        durations = [exec_true()["duration_ms"] for _ in range(20)]
        median, p95 = timed(exec_true, EXEC_RUNS)
        rows.append(
            ("`exec.run true` over the socket", f"median {median:.1f} ms, p95 {p95:.1f} ms", "")
        )
        rows.append(
            ("  of which on the SSH channel (`duration_ms`)", f"median {statistics.median(durations):.0f} ms", "")
        )

        control = bed.work / "cm"
        ssh = [
            "ssh", "-F", str(bed.work / "ssh_config"),
            "-o", f"UserKnownHostsFile={bed.work}/known_hosts",
            "-o", f"ControlPath={control}", "-o", "ControlMaster=auto", "-o", "ControlPersist=60",
            "test", "true",
        ]
        subprocess.run(ssh, check=True, stdin=subprocess.DEVNULL)
        ssh_median, ssh_p95 = timed(
            lambda: subprocess.run(ssh, check=True, stdin=subprocess.DEVNULL), CLI_RUNS
        )
        rows.append(
            ("OpenSSH `ssh test true`, multiplexed (baseline)", f"median {ssh_median:.1f} ms, p95 {ssh_p95:.1f} ms", "")
        )
        cli_median, cli_p95 = timed(lambda: bed.run("--json", "exec", "test", "--", "true"), CLI_RUNS)
        rows.append(
            ("`hostlane exec test -- true` (new CLI process)", f"median {cli_median:.1f} ms, p95 {cli_p95:.1f} ms", "")
        )
        rows.append(
            ("Overhead over multiplexed OpenSSH", f"socket {median - ssh_median:+.1f} ms, CLI {cli_median - ssh_median:+.1f} ms", "< 50 ms")
        )
        subprocess.run(["ssh", "-o", f"ControlPath={control}", "-O", "exit", "test"], capture_output=True)

        start = time.perf_counter()
        with ThreadPoolExecutor(8) as pool:
            list(pool.map(lambda alias: bed.rpc("hosts.connect", {"host": alias}), aliases))
        connect_s = time.perf_counter() - start
        connections = bed.rpc("connections.list")["connections"]
        connected = sum(1 for c in connections if c["state"] == "connected")
        time.sleep(1)
        rows.append(
            (f"{HOSTS} hosts connected", f"{connected} up in {connect_s:.1f} s, RSS {rss_mb(pid):.1f} MB", f"{HOSTS} hosts")
        )

        blob_port = blob_server()
        opened = []
        for index in range(TUNNELS):
            alias = aliases[index % len(aliases)]
            tunnel = bed.rpc("tunnels.open", {"host": alias, "remote_port": blob_port})["tunnel"]
            opened.append(tunnel)
        listed = bed.rpc("tunnels.list")["tunnels"]
        active = sum(1 for t in listed if t["state"] == "active")
        for tunnel in listed:
            if tunnel["state"] != "active":
                print(f"tunnel {tunnel['id']} {tunnel['state']}: {tunnel.get('error')}")
        time.sleep(1)
        rows.append(
            (f"{TUNNELS} tunnels open", f"{active} active, RSS {rss_mb(pid):.1f} MB", f"{TUNNELS} tunnels")
        )

        start = time.perf_counter()
        received = 0
        with socket.create_connection(("127.0.0.1", opened[0]["local_port"])) as conn:
            conn.sendall(b"g")
            while chunk := conn.recv(1 << 20):
                received += len(chunk)
        seconds = time.perf_counter() - start
        rows.append(
            ("Tunnel throughput (one connection)", f"{received / (1 << 20) / seconds:.0f} MB/s ({received >> 20} MB)", "")
        )
        time.sleep(2)
        rows.append(("Daemon memory after the run (RSS)", f"{rss_mb(pid):.1f} MB", ""))
    finally:
        bed.cleanup()

    import platform

    print(f"Hostlane {bed.hl}\n{platform.platform()}, {os.cpu_count()} CPUs\n")
    print("| Measure | Result | Target |\n| --- | --- | --- |")
    for name, result, target in rows:
        print(f"| {name} | {result} | {target} |")


if __name__ == "__main__":
    main()
