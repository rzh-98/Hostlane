import { execFile } from 'node:child_process'
import { existsSync } from 'node:fs'
import { createConnection } from 'node:net'
import { join, resolve } from 'node:path'
import { app } from 'electron'
import type { HostlaneError, HostlaneReply } from '../shared/ipc'
import type { RpcError } from '../shared/protocol/RpcError'
import { socketPath } from './paths'

const CALL_TIMEOUT_MS = 10_000
const START_TIMEOUT_MS = 15_000
const binaryName = process.platform === 'win32' ? 'hostlane.exe' : 'hostlane'

let nextId = 1

/** HOSTLANE_BIN, the copy bundled with the app, or the workspace debug build in development. */
export function binaryPath(): string {
  const override = process.env['HOSTLANE_BIN']
  if (override) return override
  if (app.isPackaged) return join(process.resourcesPath, 'bin', binaryName)
  return resolve(app.getAppPath(), '../../target/debug', binaryName)
}

function failure(
  kind: HostlaneError['kind'],
  message: string,
  hint?: string
): HostlaneReply<never> {
  return { ok: false, error: { kind, message, hint } }
}

/** One JSON-RPC round trip on a fresh connection to the daemon socket. */
export function callDaemon<T>(method: string, params?: unknown): Promise<HostlaneReply<T>> {
  const path = socketPath()
  const request = JSON.stringify({ jsonrpc: '2.0', id: nextId++, method, params: params ?? null })

  return new Promise((done) => {
    let buffer = ''
    let settled = false
    const socket = createConnection(path)
    const settle = (reply: HostlaneReply<T>) => {
      if (settled) return
      settled = true
      socket.destroy()
      done(reply)
    }

    socket.setTimeout(CALL_TIMEOUT_MS, () =>
      settle(failure('TIMEOUT', `守护进程在 ${CALL_TIMEOUT_MS / 1000} 秒内没有响应`))
    )
    socket.on('connect', () => socket.write(`${request}\n`))
    socket.on('data', (chunk) => {
      buffer += chunk.toString('utf8')
      const end = buffer.indexOf('\n')
      if (end < 0) return
      try {
        const response = JSON.parse(buffer.slice(0, end)) as { result?: T; error?: RpcError }
        if (response.error) {
          const { message, data } = response.error
          settle({ ok: false, error: { kind: data.kind, message, hint: data.hint } })
        } else {
          settle({ ok: true, result: response.result as T })
        }
      } catch {
        settle(failure('IO', '守护进程返回了无法解析的响应'))
      }
    })
    socket.on('error', (error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT' || error.code === 'ECONNREFUSED') {
        settle(
          failure(
            'DAEMON_NOT_RUNNING',
            '守护进程未运行',
            '点击“启动守护进程”，或运行 hostlane daemon start'
          )
        )
      } else {
        settle(failure('IO', error.message))
      }
    })
    socket.on('close', () => settle(failure('IO', '连接在收到响应前关闭')))
  })
}

/** Runs `hostlane daemon start`, which detaches the daemon and waits until it answers. */
export function startDaemon(): Promise<HostlaneReply<null>> {
  const bin = binaryPath()
  if (!existsSync(bin)) {
    return Promise.resolve(
      failure(
        'DAEMON_BINARY_MISSING',
        `找不到 hostlane 程序：${bin}`,
        '在仓库根目录运行 cargo build'
      )
    )
  }
  return new Promise((done) => {
    execFile(bin, ['daemon', 'start'], { timeout: START_TIMEOUT_MS }, (error, _stdout, stderr) => {
      if (error) {
        done(failure('DAEMON_START_FAILED', stderr.trim() || error.message))
      } else {
        done({ ok: true, result: null })
      }
    })
  })
}

/** Called at launch: start the daemon unless one is already listening. */
export async function ensureDaemon(): Promise<void> {
  const status = await callDaemon('daemon.status')
  if (status.ok) return
  if (status.error.kind !== 'DAEMON_NOT_RUNNING') return
  const started = await startDaemon()
  if (!started.ok) console.warn(`[hostlane] ${started.error.message}`)
}
