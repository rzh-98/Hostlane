import { spawnSync } from 'node:child_process'
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve, basename } from 'node:path'

const dir = mkdtempSync(join(tmpdir(), 'hl-tauri-'))
const windows = process.platform === 'win32'
const env = {
  ...process.env,
  HOSTLANE_DATA_DIR: join(dir, 'data'),
  HOSTLANE_SOCKET: windows ? `\\\\.\\pipe\\${basename(dir)}` : join(dir, 'sock'),
  HOSTLANE_SSH_CONFIG: join(dir, 'config'),
  HOSTLANE_NOTIFY: '0'
}
writeFileSync(env.HOSTLANE_SSH_CONFIG, 'Host tauri-smoke\n  HostName 127.0.0.1\n')
delete env.HOSTLANE_BIN
delete env.HOSTLANE_TOKEN
try {
  const result = spawnSync(resolve(process.argv[2]), ['--check-renderer'], {
    env,
    stdio: 'inherit',
    timeout: 90000
  })
  if (result.status !== 0) throw new Error(`Desktop smoke failed: ${result.error ?? result.status}`)
} finally {
  spawnSync(resolve(`src-tauri/runtime/hostlane${windows ? '.exe' : ''}`), ['daemon', 'stop'], {
    env,
    timeout: 15000
  })
  rmSync(dir, { recursive: true, force: true, maxRetries: 3 })
}
