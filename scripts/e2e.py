#!/usr/bin/env python3
"""End-to-end test: runs the hostlane CLI and MCP server against a throwaway sshd.

Usage: scripts/e2e.py [path/to/hostlane]   (default: target/debug/hostlane)

Starts sshd on 127.0.0.1 (port $E2E_PORT, default 2222); see testbed.py. Set E2E_KEEP=1 to
keep the temporary directory.
"""

import http.server
import json
import socket
import subprocess
import sys
import threading
import urllib.request

from testbed import Testbed

BED = Testbed(sys.argv[1] if len(sys.argv) > 1 else None)
HL, ENV, WORK = BED.hl, BED.env, BED.work

passed = 0


def step(name):
    global passed
    passed += 1
    print(f"ok {passed} - {name}", flush=True)


def fail(message):
    print(f"FAIL: {message}\n{BED.logs()}", file=sys.stderr)
    raise SystemExit(1)


def hl(*args, stdin=None, check=True):
    """Runs `hostlane --json ARGS`; returns the parsed result, or the error object."""
    proc = subprocess.run(
        [HL, "--json", *args],
        input=stdin if stdin is not None else "",
        capture_output=True,
        text=True,
        env=ENV,
        timeout=120,
    )
    try:
        out = json.loads(proc.stdout) if proc.stdout.strip() else None
    except json.JSONDecodeError:
        fail(f"hostlane {' '.join(args)}: not JSON: {proc.stdout!r} {proc.stderr!r}")
    if check and proc.returncode != 0:
        fail(f"hostlane {' '.join(args)} exited {proc.returncode}: {proc.stdout} {proc.stderr}")
    return out


def hl_error(*args, **kwargs):
    out = hl(*args, check=False, **kwargs)
    if not out or "error" not in out:
        fail(f"hostlane {' '.join(args)}: expected an error, got {out}")
    return out["error"]


def approved(*args, check=True):
    """Runs a call that needs approval: --async, approve the ticket, retry with it."""
    error = hl_error("--async", *args)
    expect(error["kind"] == "APPROVAL_REQUIRED", f"{args}: expected APPROVAL_REQUIRED, got {error}")
    ticket = error["details"]["ticket_id"]
    hl("approve", ticket, "--yes")
    return hl("--ticket", ticket, *args, check=check)


def expect(condition, message):
    if not condition:
        fail(message)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def serve_http():
    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            body = b"<html><head><title>e2e page</title></head><body>hi</body></html>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", free_port()), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def mcp_session(calls):
    proc = subprocess.Popen(
        [HL, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=ENV
    )

    def send(message):
        proc.stdin.write(json.dumps(message) + "\n")
        proc.stdin.flush()

    def recv():
        line = proc.stdout.readline()
        if not line:
            fail("hostlane mcp closed stdout")
        return json.loads(line)

    send(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "1"},
            },
        }
    )
    init = recv()
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    tools = [tool["name"] for tool in recv()["result"]["tools"]]
    results = []
    for index, (name, arguments) in enumerate(calls):
        send(
            {
                "jsonrpc": "2.0",
                "id": 10 + index,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }
        )
        results.append(recv()["result"])
    proc.stdin.close()
    proc.wait(timeout=10)
    return init["result"], tools, results


def main():
    print(f"# hostlane: {HL}\n# work dir: {WORK}")
    BED.start_sshd()
    BED.write_ssh_config()
    fingerprint = BED.fingerprint()
    remote = WORK / "remote"
    remote.mkdir()

    hl("daemon", "start")
    status = hl("status")
    expect(status.get("pid"), f"daemon not running: {status}")
    step("daemon starts")

    hosts = hl("hosts")["hosts"]
    test = next((h for h in hosts if h["alias"] == "test"), None)
    expect(test is not None, f"host test missing: {hosts}")
    expect(test.get("env") == "dev" and "e2e" in test.get("tags", []), f"labels: {test}")
    step("hosts lists ssh_config entries with labels")

    error = hl_error("exec", "test", "--", "true")
    expect(error["kind"] == "HOST_KEY_UNKNOWN", f"unknown key: {error}")
    step("unknown host key is refused")

    wrong = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    result = approved("trust", "test", "--fingerprint", wrong, check=False)
    expect("error" in result, f"wrong fingerprint accepted: {result}")
    approved("trust", "test", "--fingerprint", fingerprint)
    step("trust needs approval and the matching fingerprint")

    result = hl("exec", "test", "--", "echo hello; echo oops >&2; exit 3")
    expect(result["stdout"] == "hello\n", f"stdout: {result}")
    expect(result["stderr"] == "oops\n" and result["exit_code"] == 3, f"stderr/exit: {result}")
    plain = subprocess.run(
        [HL, "exec", "test", "--", "exit 7"], env=ENV, capture_output=True, stdin=subprocess.DEVNULL
    )
    expect(plain.returncode == 7, f"plain exit code {plain.returncode}")
    step("exec returns stdout, stderr and exit code")

    result = hl("exec", "test", "--max-output", "256", "--", "seq 1 5000")
    expect(result["truncated"], f"not truncated: {result['stdout_bytes']}")
    chunk = hl("output", result["output_id"], "--offset=-5")
    expect(chunk["data"].endswith("000\n"), f"output tail: {chunk}")
    step("long output is truncated and readable by output_id")

    target = remote / "app.conf"
    written = hl("write", "test", str(target), stdin="port = 8000\nname = demo\n")
    expect(target.read_text() == "port = 8000\nname = demo\n", "write content")
    read = hl("read", "test", str(target))
    expect(read["sha256"] == written["sha256"], f"sha mismatch {read} {written}")
    edited = hl("edit", "test", str(target), "--old", "8000", "--new", "8100", "--expect", read["sha256"])
    expect("8100" in target.read_text(), "edit content")
    error = hl_error("edit", "test", str(target), "--old", "8100", "--new", "1", "--expect", read["sha256"])
    expect(error["kind"] == "FILE_CHANGED", f"stale hash: {error}")
    hl("revert", edited["change_id"])
    expect("8000" in target.read_text(), "revert content")
    step("write, read, edit with hash check, revert")

    listing = hl("ls", "test", str(remote))
    expect(any(e["name"] == "app.conf" for e in listing["entries"]), f"ls: {listing}")
    found = hl("search", "test", "name = demo", str(remote), "-F")
    expect(len(found["matches"]) == 1, f"search: {found}")
    step("ls and search")

    http_port = serve_http()
    ports = hl("ports", "test")["ports"]
    entry = next((p for p in ports if p["port"] == http_port), None)
    expect(entry is not None, f"port {http_port} not found in {[p['port'] for p in ports]}")
    expect(entry.get("http", {}).get("title") == "e2e page", f"probe: {entry}")
    check = hl("port-check", "test", str(http_port))
    expect(check["in_use"], f"port-check: {check}")
    hl("port-note", "test", str(http_port), "--name", "e2e", "--project", "hostlane")
    step("ports discovers and probes the web server")

    opened = hl("tunnel", "open", "test", str(http_port))["tunnel"]
    with urllib.request.urlopen(opened["url"], timeout=10) as response:
        expect(b"e2e page" in response.read(), "tunnel body")
    tunnels = hl("tunnels")["tunnels"]
    expect(any(t["id"] == opened["id"] and t["state"] == "active" for t in tunnels), f"{tunnels}")
    hl("tunnel", "close", opened["id"])
    step("tunnel forwards a local port")

    error = hl_error("--async", "exec", "test", "--", "sudo -n true")
    expect(error["kind"] == "APPROVAL_REQUIRED", f"approval: {error}")
    ticket = error["details"]["ticket_id"]
    pending = hl("approvals")["approvals"]
    expect(any(a["id"] == ticket for a in pending), f"pending: {pending}")
    hl("approve", ticket, "--yes")
    result = hl("--ticket", ticket, "exec", "test", "--", "sudo -n true")
    expect(result["status"] == "exited", f"approved exec: {result}")
    error = hl_error("--ticket", ticket, "exec", "test", "--", "sudo -n true")
    expect(error["kind"] in ("APPROVAL_NOT_FOUND", "POLICY_DENIED", "INVALID_PARAMS"), f"reuse: {error}")
    step("L2 command waits for approval; ticket is single-use")

    hl("stop", "--reason", "e2e")
    error = hl_error("exec", "test", "--", "true")
    expect(error["kind"] == "POLICY_DENIED", f"stopped: {error}")
    hl("resume")
    hl("exec", "test", "--", "true")
    step("emergency stop refuses agent calls until resume")

    init, tools, results = mcp_session(
        [
            ("exec", {"host": "test", "command": "echo from-mcp"}),
            ("read_file", {"host": "test", "path": str(target)}),
            ("exec", {"host": "nope", "command": "true"}),
        ]
    )
    expect(init["serverInfo"]["name"] == "hostlane", f"mcp init: {init}")
    expect({"exec", "read_file", "open_tunnel"} <= set(tools), f"tools: {tools}")
    expect(results[0]["structuredContent"]["stdout"] == "from-mcp\n", f"mcp exec: {results[0]}")
    expect("port = 8000" in results[1]["structuredContent"]["content"], f"mcp read: {results[1]}")
    expect(results[2].get("isError"), f"mcp error: {results[2]}")
    expect(
        results[2]["structuredContent"]["error"]["kind"] == "HOST_NOT_FOUND", f"{results[2]}"
    )
    step("MCP server exposes tools that go through the daemon")

    audit = hl("audit", "--host", "test")["entries"]
    expect(len(audit) >= 10, f"audit entries: {len(audit)}")
    expect(any(e["client"]["name"] == "mcp" for e in audit), "mcp calls not audited")
    step("calls are audited with their client")

    hl("daemon", "stop")
    step("daemon stops")


if __name__ == "__main__":
    try:
        main()
    finally:
        BED.cleanup()
    print(f"# {passed} passed")
