#!/usr/bin/env python3
"""End-to-end test: runs the hostlane CLI and MCP server against a throwaway sshd.

Usage: scripts/e2e.py [path/to/hostlane]   (default: target/debug/hostlane)

Starts sshd on 127.0.0.1 (port $E2E_PORT, default 2222); see testbed.py. Set E2E_KEEP=1 to
keep the temporary directory.
"""

import csv
import http.server
import io
import json
import shutil
import socket
import subprocess
import sys
import time
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


def hl(*args, stdin=None, check=True, env=None):
    """Runs `hostlane --json ARGS`; returns the parsed result, or the error object."""
    proc = subprocess.run(
        [HL, "--json", *args],
        input=stdin if stdin is not None else "",
        capture_output=True,
        text=True,
        env={**ENV, **(env or {})},
        timeout=180,
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


def maybe_approved(*args):
    """Runs a call that may need approval depending on the policy."""
    out = hl("--async", *args, check=False)
    if out and out.get("error", {}).get("kind") == "APPROVAL_REQUIRED":
        ticket = out["error"]["details"]["ticket_id"]
        hl("approve", ticket, "--yes")
        out = hl("--ticket", ticket, *args, check=False)
    if not out or "error" in out:
        fail(f"hostlane {' '.join(args)}: {out}")
    return out


def expect(condition, message):
    if not condition:
        fail(message)


def await_reachability(host, expected):
    deadline = time.monotonic() + 12
    rows = []
    while time.monotonic() < deadline:
        rows = hl("host", "reachability")
        if any(row["host"] == host and row["status"] == expected and row["checked_at"] > 0 for row in rows):
            return
        time.sleep(0.1)
    fail(f"expected {host}: {expected}, got {rows}")


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

    ENV["HOSTLANE_SECRET_STORE"] = "file"
    hl("daemon", "start")
    status = hl("status")
    expect(status.get("pid"), f"daemon not running: {status}")
    step("daemon starts")

    hosts = hl("hosts")["hosts"]
    test = next((h for h in hosts if h["alias"] == "test"), None)
    expect(test is not None, f"host test missing: {hosts}")
    expect(test.get("env") == "dev" and "e2e" in test.get("tags", []), f"labels: {test}")
    step("hosts lists ssh_config entries with labels")

    expect(hl("hosts")["reachability"] == [], "monitoring must default to off")
    enable = json.dumps({"host": "test", "enabled": True})
    disable = json.dumps({"host": "test", "enabled": False})
    error = hl_error("call", "reachability.set", enable, env={"HOSTLANE_CLIENT": "agent"})
    expect(error["kind"] == "POLICY_DENIED", f"agent must not enable monitoring: {error}")
    hl("host", "monitor", "test", "on")
    await_reachability("test", "ssh_banner")
    expect(hl("connections")["connections"] == [], "banner probe established a pooled session")
    hl("daemon", "stop")
    hl("daemon", "start")
    await_reachability("test", "ssh_banner")
    _, _, cached = mcp_session([("hosts", {})])
    expect(cached[0]["structuredContent"]["reachability"][0]["status"] == "ssh_banner", "MCP hosts missing sample")
    hl("call", "reachability.set", disable)
    expect(hl("hosts")["reachability"] == [], "disabled monitor still listed")
    hl("host", "add", "via-test", "127.0.0.1", "--port", str(BED.port), "--jump", "test")
    hl("call", "reachability.set", json.dumps({"host": "via-test", "enabled": True}))
    await_reachability("via-test", "jump_disconnected")
    expect(hl("connections")["connections"] == [], "monitor tried to log in to the jump host")
    hl("host", "remove", "via-test")
    expect(hl("hosts")["reachability"] == [], "removed host still monitored")
    step("SSH reachability is opt-in, survives restart and does not log in")

    error = hl_error("exec", "test", "--", "true")
    expect(error["kind"] == "HOST_KEY_UNKNOWN", f"unknown key: {error}")
    step("unknown host key is refused")

    wrong = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    result = approved("trust", "test", "--fingerprint", wrong, check=False)
    expect("error" in result, f"wrong fingerprint accepted: {result}")
    approved("trust", "test", "--fingerprint", fingerprint)
    step("trust needs approval and the matching fingerprint")

    hl("call", "reachability.set", enable)
    await_reachability("test", "session_alive")
    hl("call", "reachability.set", disable)
    step("SSH reachability reuses the established session")

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

    tagged = {"HOSTLANE_TASK": "deploy-e2e", "HOSTLANE_SESSION": "review-e2e"}
    note = str(remote / "rollback.txt")
    hl("write", "test", note, stdin="one", env=tagged)
    hl("edit", "test", note, "--old", "one", "--new", "two", env=tagged)
    hl("exec", "test", "--env", "HL_SAMPLE=e2e-private-value", "--", 'test -n "$HL_SAMPLE" && echo recorded', env=tagged)
    timeline = hl("timeline", "--task", "deploy-e2e")
    expect(len(timeline) == 3 and timeline[0]["method"] == "files.write", f"timeline: {timeline}")
    replay = hl("replay", "--session", "review-e2e", "--command", "1")
    expect(replay["stdout"]["data"] == "recorded\n", f"replay: {replay}")
    exported = hl("audit-export", "--task", "deploy-e2e")["data"].splitlines()
    expect([json.loads(row)["id"] for row in exported] == [row["id"] for row in reversed(timeline)], "JSONL export")
    csv_export = hl("audit-export", "--format", "csv", "--task", "deploy-e2e")["data"]
    csv_rows = list(csv.DictReader(io.StringIO(csv_export)))
    expect([int(row['id']) for row in csv_rows] == [row['id'] for row in reversed(timeline)], "CSV export order")
    expect(json.loads(csv_rows[0]['params'])['env']['HL_SAMPLE'] == '<redacted>' and 'e2e-private-value' not in csv_export + ''.join(exported), "environment value in audit export")
    plan = hl("rollback", "deploy-e2e")
    expect(len(plan) == 2 and len(hl("rollback", "deploy-e2e", "--apply")["reverted"]) == 2, "task rollback")
    expect(not (remote / "rollback.txt").exists() and not hl("rollback", "deploy-e2e"), "rollback must be resumable")
    step("task timeline, replay, export and reverse-order file rollback")

    older, newer = remote / "older.txt", remote / "newer.txt"
    older_change = hl("write", "test", str(older), stdin="original", env=tagged)
    hl("write", "test", str(newer), stdin="recent", env=tagged)
    older.write_text("external update")
    expect(hl_error("rollback", "deploy-e2e", "--apply")["kind"] == "FILE_CHANGED", "batch must stop on conflict")
    expect(not newer.exists() and older.read_text() == "external update", "rollback overwrote a conflict")
    remaining = hl("rollback", "deploy-e2e")
    expect([item['change_id'] for item in remaining] == [older_change['change_id']], f"incorrect remaining plan: {remaining}")
    older.write_text("original")
    expect(len(hl("rollback", "deploy-e2e", "--apply")["reverted"]) == 1 and not older.exists(), "resume after conflict")
    step("partial rollback preserves external changes and resumes from the remaining item")

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

    profile = hl("web", "profile", "test")
    expect(profile["system"] and int(profile["cpu"]) > 0, f"profile: {profile}")
    hl("web", "note", "test", stdin="Maintenance window: Friday\n")
    context = hl("web", "export", "test")
    expect("Maintenance window: Friday" in context["markdown"], f"context: {context}")
    report = hl("web", "report", "test")
    expect(any(item["port"] == http_port and item["assessment"] == "loopback" for item in report["listeners"]), "loopback exposure not identified")
    step("profile, Markdown notes and listener exposure")

    hl("web", "monitor", "test", str(http_port), "10")
    for _ in range(25):
        health = hl("web", "health")
        if health and health[0]["healthy"] is not None:
            break
        time.sleep(1)
    expect(health and health[0]["healthy"], f"health check: {health}")
    unavailable_port = str(free_port())
    hl("web", "monitor", "test", unavailable_port, "10")
    hl("daemon", "stop")
    hl("daemon", "start")
    for _ in range(30):
        health = hl("web", "health")
        if any(row["port"] == int(unavailable_port) and row["healthy"] is False for row in health):
            break
        time.sleep(1)
    expect(any(row["port"] == int(unavailable_port) and row["healthy"] is False for row in health), "failed service not detected after restart")
    hl("web", "monitor", "test", unavailable_port, "0")
    hl("web", "monitor", "test", str(http_port), "0")
    expect(hl("web", "health") == [], "monitor not removed")
    step("daemon schedules an HTTP health check")

    reverse = approved("web", "reverse", "test", "0", str(http_port))
    assigned = reverse["remote_port"]
    expect(reverse in hl("web", "forwards"), "missing reverse listener")
    with urllib.request.urlopen(f"http://127.0.0.1:{assigned}/", timeout=10) as response:
        expect(b"e2e page" in response.read(), "reverse forward body")
    hl("web", "unforward", "test", str(assigned))
    expect(hl("web", "forwards") == [], "reverse forward not removed")
    with socket.socket() as sock:
        expect(sock.connect_ex(("127.0.0.1", assigned)) != 0, "closed reverse port still listening")
    step("approved reverse forward transfers HTTP and closes")

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

    passphrase = "e2e-pass-3141"
    locked = WORK / "id_locked"
    subprocess.run(
        ["ssh-keygen", "-q", "-t", "ed25519", "-N", passphrase, "-f", str(locked)], check=True
    )
    with open(WORK / "authorized_keys", "a") as keys:
        keys.write((WORK / "id_locked.pub").read_text())
    added = hl(
        "host", "add", "manual", "127.0.0.1", "--port", str(BED.port), "--user", BED.user,
        "--key", str(locked), "--env", "dev", "--tag", "e2e",
    )
    expect(added["source"] == "manual" and added["tags"] == ["e2e"], f"host add: {added}")
    error = hl_error("host", "add", "manual", "127.0.0.2")
    expect(error["kind"] == "INVALID_PARAMS", f"duplicate host: {error}")
    error = hl_error("exec", "manual", "--", "true")
    expect(error["kind"] == "AUTH_FAILED", f"locked key: {error}")
    expect(error["details"]["encrypted_keys"], f"auth details: {error}")
    error = hl_error("secret", "set", "manual", "--passphrase", "--stdin", stdin="wrong\n")
    expect(error["kind"] == "INVALID_PARAMS", f"wrong passphrase: {error}")
    hl("secret", "set", "manual", "--passphrase", "--stdin", stdin=passphrase + "\n")
    result = hl("exec", "manual", "--", "echo unlocked")
    expect(result["stdout"] == "unlocked\n", f"passphrase login: {result}")
    secrets = hl("secrets")
    expect(secrets["store"] == "file" and len(secrets["secrets"]) == 1, f"secrets: {secrets}")
    expect(passphrase not in json.dumps(secrets), "secrets.list leaks the secret")
    updated = hl("host", "update", "manual", "--tag", "renamed")
    expect(updated["tags"] == ["renamed"], f"host update: {updated}")
    expect(hl("exec", "manual", "--", "true")["exit_code"] == 0, "exec after update")
    removed = hl("host", "remove", "manual")
    expect(removed["host"] == "manual", f"host remove: {removed}")
    expect(all(h["id"] != "manual" for h in hl("hosts")["hosts"]), "removed host listed")
    error = hl_error("host", "remove", "test")
    expect(error["kind"] == "INVALID_PARAMS", f"remove ssh_config host: {error}")
    step("manual hosts and saved key passphrases")

    started = hl("exec", "test", "--", "sh -c 'sleep 300 </dev/null >/dev/null 2>&1 & echo $!'")
    pid = started["stdout"].strip()
    expect(pid.isdigit(), f"sleep pid: {started}")
    killed = maybe_approved("kill", "test", pid)
    expect(killed["exit_code"] == 0 and killed["command"].startswith("kill -TERM"), f"{killed}")
    alive = hl("exec", "test", "--", f"sleep 0.2; kill -0 {pid}")
    expect(alive["exit_code"] != 0, f"process still alive: {alive}")
    logs = maybe_approved("logs", "test", "hostlane-e2e-missing", "-n", "5")
    expect("journalctl" in logs["command"], f"logs command: {logs}")
    logs = maybe_approved("logs", "test", "web", "--docker", "--since", "10m")
    expect(logs["command"].startswith("docker logs"), f"docker logs command: {logs}")
    error = hl_error("logs", "test", "a;b")
    expect(error["kind"] == "INVALID_PARAMS", f"bad unit name: {error}")
    step("processes.kill and services.logs build and run checked commands")

    script = "for i in 1 2 3; do echo line$i; sleep 1; done; echo done >&2; exit 4"
    job = maybe_approved("job", "start", "test", "--name", "count", "--", script)
    expect(job["state"] == "running" and job["backend"] in ("tmux", "nohup"), f"job: {job}")
    hl("disconnect", "test")
    status = hl("job", "status", job["id"], "--wait", "30")
    expect(status["state"] == "exited" and status["exit_code"] == 4, f"job status: {status}")
    logs = maybe_approved("job", "logs", job["id"])
    expect(logs["data"] == "line1\nline2\nline3\ndone\n", f"job logs: {logs}")
    expect(logs["complete"] and logs["untrusted"] == ["data"], f"job logs flags: {logs}")
    part = maybe_approved("job", "logs", job["id"], "--offset", "6")
    expect(part["data"] == "line2\nline3\ndone\n" and part["offset"] == 6, f"offset: {part}")
    follow = subprocess.run(
        [HL, "job", "logs", job["id"], "--follow"], env=ENV, capture_output=True, text=True,
        stdin=subprocess.DEVNULL, timeout=60,
    )
    expect(follow.returncode == 4 and "line3" in follow.stdout, f"follow: {follow}")
    long = maybe_approved("job", "start", "test", "--nohup", "--", "sleep 301 & sleep 301")
    expect(long["backend"] == "nohup" and long.get("pid"), f"nohup job: {long}")
    cancelled = maybe_approved("job", "cancel", long["id"])
    expect(cancelled["state"] == "cancelled", f"cancel: {cancelled}")
    left = hl("exec", "test", "--", "ps -eo args= | grep -c '^sleep 30[1]$' || true")
    expect(left["stdout"].strip() == "0", f"job processes left: {left}")
    if shutil.which("tmux"):
        in_tmux = maybe_approved("job", "start", "test", "--tmux", "--", "echo in-tmux; exit 2")
        expect(in_tmux["backend"] == "tmux", f"tmux job: {in_tmux}")
        status = hl("job", "status", in_tmux["id"], "--wait", "30")
        expect(status["state"] == "exited" and status["exit_code"] == 2, f"tmux status: {status}")
        logs = maybe_approved("job", "logs", in_tmux["id"])
        expect(logs["data"] == "in-tmux\n", f"tmux logs: {logs}")
        held = maybe_approved("job", "start", "test", "--tmux", "--", "sleep 303 & sleep 303")
        cancelled = maybe_approved("job", "cancel", held["id"])
        expect(cancelled["state"] == "cancelled", f"tmux cancel: {cancelled}")
        left = hl("exec", "test", "--", "ps -eo args= | grep -c '^sleep 30[3]$' || true")
        expect(left["stdout"].strip() == "0", f"tmux job processes left: {left}")
        print("# tmux backend tested", flush=True)
    third = maybe_approved("job", "start", "test", "--", "sleep 302")
    stopped = hl("stop", "--reason", "e2e jobs")
    expect(third["id"] in stopped["cancelled_jobs"], f"stop did not cancel the job: {stopped}")
    hl("resume")
    jobs = hl("jobs")["jobs"]
    expect({job["id"], long["id"], third["id"]} <= {j["id"] for j in jobs}, f"jobs: {jobs}")
    expect(all(j["state"] != "running" for j in jobs), f"running jobs left: {jobs}")
    step("background jobs survive disconnects, stream logs and cancel")

    token = hl("client", "add", "e2e-agent")["token"]
    agent = {"HOSTLANE_TOKEN": token, "HOSTLANE_CLIENT": "pretender"}
    hl("exec", "test", "--", "echo as-agent", env=agent)
    latest = hl("audit", "--limit", "1")["entries"][0]
    expect(latest["client"]["name"] == "e2e-agent", f"token client: {latest}")
    expect(latest["client"].get("verified") is True, f"not verified: {latest}")
    hl("client", "require-token", "on")
    error = hl_error("exec", "test", "--", "true", env={"HOSTLANE_CLIENT": "anon"})
    expect(error["kind"] == "UNAUTHORIZED", f"anonymous agent: {error}")
    hl("exec", "test", "--", "true")
    hl("client", "revoke", "e2e-agent")
    error = hl_error("exec", "test", "--", "true", env=agent)
    expect(error["kind"] == "UNAUTHORIZED", f"revoked token: {error}")
    hl("client", "require-token", "off")
    expect(token not in json.dumps(hl("audit", "--limit", "500")), "token leaked into the audit log")
    step("client tokens identify agents; revoked or missing tokens are refused")

    init, tools, results = mcp_session(
        [
            ("exec", {"host": "test", "command": "echo from-mcp"}),
            ("read_file", {"host": "test", "path": str(target)}),
            ("exec", {"host": "nope", "command": "true"}),
        ]
    )
    expect(init["serverInfo"]["name"] == "hostlane", f"mcp init: {init}")
    expected_tools = {"exec", "read_file", "open_tunnel", "service_logs", "job_start", "job_logs"}
    expect(expected_tools <= set(tools), f"tools: {tools}")
    expect(not {"add_host", "set_secret"} & set(tools), f"person-only tools: {tools}")
    expect(results[0]["structuredContent"]["stdout"] == "from-mcp\n", f"mcp exec: {results[0]}")
    expect("Untrusted data" in results[0]["content"][0]["text"], f"mcp untrusted: {results[0]}")
    expect("untrusted" in init["instructions"], f"mcp instructions: {init}")
    expect("port = 8000" in results[1]["structuredContent"]["content"], f"mcp read: {results[1]}")
    expect(results[2].get("isError"), f"mcp error: {results[2]}")
    expect(
        results[2]["structuredContent"]["error"]["kind"] == "HOST_NOT_FOUND", f"{results[2]}"
    )
    step("MCP server exposes tools that go through the daemon")

    audit = hl("audit", "--host", "test")["entries"]
    expect(len(audit) >= 10, f"audit entries: {len(audit)}")
    expect(any(e["client"]["name"] == "mcp" for e in audit), "mcp calls not audited")
    everything = json.dumps(hl("audit", "--limit", "500"))
    expect("secrets.set" in everything and "<redacted>" in everything, "secrets.set not audited")
    log = (WORK / "data" / "daemon.log").read_text() if (WORK / "data" / "daemon.log").exists() else ""
    expect(passphrase not in everything and passphrase not in log, "passphrase leaked")
    step("calls are audited with their client; secrets are redacted")

    hl("daemon", "stop")
    step("daemon stops")


if __name__ == "__main__":
    try:
        main()
    finally:
        BED.cleanup()
    print(f"# {passed} passed")
