"""Run against a GUI login on macOS, or under dbus-run-session on Linux."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def wait_for(probe, description):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        result = probe()
        if result:
            return result
        time.sleep(0.1)
    raise RuntimeError(f"Timed out: {description}")


def smoke(work):
    cli = work / 'resources/bin/hostlane'
    cli.parent.mkdir(parents=True)
    shutil.copy2(sys.argv[1], cli)
    env = dict(os.environ, HOSTLANE_SOCKET=str(work / 'sock'), HOSTLANE_DATA_DIR=str(work / 'data'),
               HOSTLANE_SSH_CONFIG=str(work / 'ssh_config'), HOSTLANE_NOTIFY='0')
    (work / 'ssh_config').touch()
    linux = sys.platform == 'linux'
    if linux:
        assert env.get('DBUS_SESSION_BUS_ADDRESS'), 'Use dbus-run-session for an isolated session bus'
        env.pop('APPDIR', None)
        env.pop('APPIMAGE', None)
        env['TRAY_TEST_MARKER'] = str(work / 'opened')
        desktop = work / 'hostlane-desktop'
        desktop.write_text('#!/bin/sh\ntouch "$TRAY_TEST_MARKER"\n')
        desktop.chmod(0o700)

    def run(*args):
        return subprocess.run(args, env=env, capture_output=True, text=True, timeout=5)

    daemon = subprocess.Popen([cli, 'daemon', 'run'], env=env)
    try:
        wait_for(lambda: run(cli, '--json', 'status').returncode == 0, 'daemon startup')
        assert json.loads(run(cli, '--json', 'status').stdout)['pid'] == daemon.pid
        if linux:
            names = lambda: run('busctl', '--user', '--no-pager', '--no-legend', 'list').stdout.split()
            prefix = f'org.kde.StatusNotifierItem-{daemon.pid}-'
            name = wait_for(lambda: next((entry for entry in names() if entry.startswith(prefix)), None), 'SNI registration')
            item = (name, '/StatusNotifierItem', 'org.kde.StatusNotifierItem')
            assert 'hostlane' in run('busctl', '--user', 'get-property', *item, 'Id').stdout
            assert '32 32 4096' in run('busctl', '--user', 'get-property', *item, 'IconPixmap').stdout
            run('busctl', '--user', 'call', *item, 'Activate', 'ii', '0', '0').check_returncode()
            wait_for((work / 'opened').exists, 'opening the bundled desktop')
            run(cli, 'daemon', 'stop').check_returncode()
            daemon.wait(timeout=5)
            assert name not in names()
        else:
            def helper():
                for line in run('ps', '-axo', 'pid=,ppid=,command=').stdout.splitlines():
                    fields = line.strip().split(None, 2)
                    if len(fields) == 3 and fields[1] == str(daemon.pid) and fields[2].endswith(' --hostlane-menu-bar'):
                        return fields[0]
                return None

            pid = wait_for(helper, 'AppKit helper startup')
            time.sleep(1)
            assert helper() == pid, 'No menu bar in this login session'
            daemon.kill()
            daemon.wait(timeout=5)
            wait_for(lambda: run('ps', '-p', pid).returncode != 0, 'menu bar cleanup after daemon crash')
    finally:
        if daemon.poll() is None:
            daemon.kill()
            daemon.wait(timeout=5)


with tempfile.TemporaryDirectory(prefix='hl-tray-', dir='/tmp') as directory:
    smoke(Path(directory))
print('Tray smoke test passed')
