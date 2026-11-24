import { execFile } from 'node:child_process'
import { existsSync } from 'node:fs'
import { lstat, mkdir, readlink, symlink, unlink } from 'node:fs/promises'
import { homedir } from 'node:os'
import { delimiter, dirname, join, resolve } from 'node:path'
import type { HostlaneError, HostlaneReply, ServiceStatus, SetupStatus } from '../shared/ipc'
import { binaryPath } from './daemon'

const SERVICE_TIMEOUT_MS = 20_000
const SHELL_TIMEOUT_MS = 3_000
/** Passed on to MCP configs so a custom or test setup reaches the same daemon. */
const HOSTLANE_ENV = ['HOSTLANE_SOCKET', 'HOSTLANE_DATA_DIR', 'HOSTLANE_SSH_CONFIG']

/** `~/.local/bin/hostlane`: user-writable, and on PATH in most shells. */
function linkPath(): string {
  return join(homedir(), '.local', 'bin', 'hostlane')
}

/** Runs `hostlane --json …` and returns its JSON result or structured error. */
function runCli<T>(args: string[], timeout: number): Promise<HostlaneReply<T>> {
  const bin = binaryPath()
  if (!existsSync(bin)) {
    return Promise.resolve({
      ok: false,
      error: { kind: 'DAEMON_BINARY_MISSING', message: `找不到 hostlane 程序：${bin}` }
    })
  }
  return new Promise((done) => {
    execFile(bin, ['--json', ...args], { timeout }, (error, stdout, stderr) => {
      let parsed: unknown = null
      try {
        parsed = JSON.parse(stdout)
      } catch {
        parsed = null
      }
      const reported = (parsed as { error?: HostlaneError } | null)?.error
      if (error || reported) {
        done({
          ok: false,
          error: reported ?? { kind: 'IO', message: stderr.trim() || (error?.message ?? '') }
        })
      } else {
        done({ ok: true, result: parsed as T })
      }
    })
  })
}

/** PATH of the user's login shell; apps started from the Dock see a shorter one. */
function shellPath(): Promise<string> {
  if (process.platform === 'win32') return Promise.resolve(process.env['PATH'] ?? '')
  const shell = process.env['SHELL'] || '/bin/sh'
  return new Promise((done) => {
    execFile(
      shell,
      ['-ilc', 'printf "__HL__%s__HL__" "$PATH"'],
      { timeout: SHELL_TIMEOUT_MS },
      (_error, stdout) => {
        const match = /__HL__(.*)__HL__/.exec(stdout)
        done(match ? match[1] : (process.env['PATH'] ?? ''))
      }
    )
  })
}

async function linkTarget(path: string): Promise<string | null> {
  try {
    const stat = await lstat(path)
    if (!stat.isSymbolicLink()) return path
    return resolve(dirname(path), await readlink(path))
  } catch {
    return null
  }
}

export async function setupStatus(): Promise<SetupStatus> {
  const binary = binaryPath()
  const link = linkPath()
  const supported = process.platform !== 'win32'
  const [target, path, service] = await Promise.all([
    supported ? linkTarget(link) : Promise.resolve(null),
    shellPath(),
    runCli<ServiceStatus>(['daemon', 'service'], SERVICE_TIMEOUT_MS)
  ])
  const linked = target === binary
  const env: Record<string, string> = {}
  for (const name of HOSTLANE_ENV) {
    const value = process.env[name]
    if (value) env[name] = value
  }
  return {
    binary,
    binaryExists: existsSync(binary),
    command: linked ? link : binary,
    env,
    cli: {
      supported,
      linkPath: link,
      linkTarget: target,
      linked,
      dirOnPath: path.split(delimiter).includes(dirname(link))
    },
    service: service.ok ? service.result : null
  }
}

/** Points `~/.local/bin/hostlane` at this app's binary, replacing an older link. */
export async function linkCli(): Promise<HostlaneReply<SetupStatus>> {
  const link = linkPath()
  const binary = binaryPath()
  try {
    if (!existsSync(binary)) throw new Error(`找不到 hostlane 程序：${binary}`)
    const current = await linkTarget(link)
    if (current !== null && current === link) {
      throw new Error(`${link} 已存在且不是符号链接，请先手动移走`)
    }
    await mkdir(dirname(link), { recursive: true })
    if (current !== null) await unlink(link)
    await symlink(binary, link)
    return { ok: true, result: await setupStatus() }
  } catch (error) {
    return { ok: false, error: { kind: 'IO', message: (error as Error).message } }
  }
}

export function installService(): Promise<HostlaneReply<unknown>> {
  return runCli(['daemon', 'install'], SERVICE_TIMEOUT_MS)
}

export function uninstallService(): Promise<HostlaneReply<unknown>> {
  return runCli(['daemon', 'uninstall'], SERVICE_TIMEOUT_MS)
}
