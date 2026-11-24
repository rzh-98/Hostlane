"""A throwaway sshd on 127.0.0.1 plus an isolated Hostlane environment, for e2e.py and bench.py.

sshd runs as the current user with its own host key, user key and config in a temporary
directory. HOSTLANE_SOCKET, HOSTLANE_DATA_DIR and HOSTLANE_SSH_CONFIG point there too, so the
real daemon and ~/.ssh are not touched.
"""

import json
import os
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class Testbed:
    def __init__(self, hostlane=None, port=None):
        self.hl = str(Path(hostlane or ROOT / "target/debug/hostlane").resolve())
        self.port = int(port or os.environ.get("E2E_PORT", "2222"))
        self.work = Path(tempfile.mkdtemp(prefix="hostlane-e2e-"))
        self.user = (
            os.environ.get("USER") or subprocess.check_output(["id", "-un"], text=True).strip()
        )
        self.env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith("HOSTLANE_") and k != "SSH_AUTH_SOCK"
        }
        self.env.update(
            HOSTLANE_SOCKET=str(self.work / "hl.sock"),
            HOSTLANE_DATA_DIR=str(self.work / "data"),
            HOSTLANE_SSH_CONFIG=str(self.work / "ssh_config"),
            HOSTLANE_NOTIFY="0",
        )

    # sshd

    def start_sshd(self, extra=""):
        sshd = shutil.which("sshd") or "/usr/sbin/sshd"
        for name in ("hostkey", "id_ed25519"):
            subprocess.run(
                ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.work / name)],
                check=True,
            )
        shutil.copy(self.work / "id_ed25519.pub", self.work / "authorized_keys")
        (self.work / "sshd_config").write_text(
            f"""Port {self.port}
ListenAddress 127.0.0.1
HostKey {self.work}/hostkey
AuthorizedKeysFile {self.work}/authorized_keys
PidFile {self.work}/sshd.pid
UsePAM no
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowTcpForwarding yes
MaxStartups 200
MaxSessions 100
Subsystem sftp internal-sftp
{extra}"""
        )
        subprocess.run(
            [sshd, "-f", str(self.work / "sshd_config"), "-E", str(self.work / "sshd.log")],
            check=True,
        )
        for _ in range(50):
            try:
                socket.create_connection(("127.0.0.1", self.port), timeout=1).close()
                return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError("sshd did not start")

    def stop_sshd(self):
        pid_file = self.work / "sshd.pid"
        if pid_file.exists():
            try:
                os.kill(int(pid_file.read_text()), 15)
            except (ProcessLookupError, ValueError):
                pass

    def fingerprint(self):
        out = subprocess.check_output(
            ["ssh-keygen", "-lf", str(self.work / "hostkey.pub")], text=True
        )
        return out.split()[1]

    def write_ssh_config(self, aliases=("test",), extra=""):
        blocks = [
            f"""Host {alias}
  # hostlane: env=dev tags=e2e
  HostName 127.0.0.1
  Port {self.port}
  User {self.user}
  IdentityFile {self.work}/id_ed25519
  IdentitiesOnly yes
"""
            for alias in aliases
        ]
        (self.work / "ssh_config").write_text("\n".join(blocks) + extra)

    def trust_in_known_hosts(self):
        """Records the host key in the known_hosts next to ssh_config, which Hostlane reads."""
        key = (self.work / "hostkey.pub").read_text().split()[:2]
        (self.work / "known_hosts").write_text(f"[127.0.0.1]:{self.port} {' '.join(key)}\n")

    # hostlane

    def run(self, *args, stdin=None):
        return subprocess.run(
            [self.hl, *args],
            input=stdin if stdin is not None else "",
            capture_output=True,
            text=True,
            env=self.env,
            timeout=120,
        )

    def rpc(self, method, params=None, client="bench"):
        """One JSON-RPC call over the daemon socket; returns the result or raises."""
        with socket.socket(socket.AF_UNIX) as sock:
            sock.connect(self.env["HOSTLANE_SOCKET"])
            request = {"jsonrpc": "2.0", "id": 1, "method": method, "client": {"name": client}}
            if params is not None:
                request["params"] = params
            sock.sendall((json.dumps(request) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = sock.recv(1 << 16)
                if not chunk:
                    break
                data += chunk
        response = json.loads(data)
        if "error" in response:
            raise RuntimeError(f"{method}: {response['error']}")
        return response["result"]

    def cleanup(self):
        subprocess.run([self.hl, "daemon", "stop"], env=self.env, capture_output=True)
        self.stop_sshd()
        if os.environ.get("E2E_KEEP") != "1":
            shutil.rmtree(self.work, ignore_errors=True)

    def logs(self):
        out = []
        for name, path in (
            ("sshd.log", self.work / "sshd.log"),
            ("daemon.log", self.work / "data" / "daemon.log"),
        ):
            if path.exists():
                out.append(f"--- {name} ---\n{path.read_text()[-4000:]}")
        return "\n".join(out)
