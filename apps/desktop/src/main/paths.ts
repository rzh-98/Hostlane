import { homedir } from 'node:os'
import { createHash } from 'node:crypto'
import { join } from 'node:path'

/** Mirrors crates/hostlane-core/src/paths.rs; change both together. */
export function dataDir(): string {
  const override = process.env['HOSTLANE_DATA_DIR']
  if (override) return override
  if (process.platform === 'darwin') return join(homedir(), 'Library/Application Support/Hostlane')
  if (process.platform === 'win32') {
    return join(process.env['APPDATA'] ?? join(homedir(), 'AppData/Roaming'), 'Hostlane')
  }
  return join(process.env['XDG_STATE_HOME'] ?? join(homedir(), '.local/state'), 'hostlane')
}

export function socketPath(): string {
  const override = process.env['HOSTLANE_SOCKET']
  if (override) return override
  const runtime = process.env['XDG_RUNTIME_DIR']
  if (process.platform === 'win32')
    return (
      '\\\\.\\pipe\\hostlane-' + createHash('sha256').update(dataDir()).digest('hex').slice(0, 24)
    )
  if (process.platform === 'linux' && runtime) return join(runtime, 'hostlane', 'hostlane.sock')
  return join(dataDir(), 'hostlane.sock')
}
