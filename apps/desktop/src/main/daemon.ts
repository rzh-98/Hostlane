import { execFile, spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { createConnection } from 'node:net'
import { Duplex } from 'node:stream'
import { join, resolve } from 'node:path'
import { app } from 'electron'
import type { HostlaneError, HostlaneEvent, HostlaneReply } from '../shared/ipc'
import type { Event } from '../shared/protocol/Event'
import type { RpcError } from '../shared/protocol/RpcError'
import { socketPath } from './paths'

const CALL_TIMEOUT_MS = 10_000
/** Connecting may wait for the SSH handshake and every jump host. */
const SLOW_CALL_TIMEOUT_MS = 60_000
const SLOW_METHODS = new Set([
  'hosts.connect',
  'hosts.trust',
  'files.revert',
  'ports.list',
  'web.report',
  'hosts.profile',
  'reverse.open',
  'ports.inspect',
  'ports.check',
  'ports.find_free',
  'tunnels.open',
  'tunnels.start',
  'tunnels.update',
  'policy.stop'
])
const START_TIMEOUT_MS = 15_000
const RESUBSCRIBE_MIN_MS = 1_000
const RESUBSCRIBE_MAX_MS = 10_000
const binaryName = process.platform === 'win32' ? 'hostlane.exe' : 'hostlane'
/** Shown as the caller in the audit log. */
const CLIENT = { name: 'desktop' }

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

function transport(path: string): Duplex {
  if (process.platform !== 'win32') return createConnection(path)
  const child = spawn(binaryPath(), ['daemon', 'exchange', '--socket', path], {
    stdio: ['pipe', 'pipe', 'ignore'],
    windowsHide: true
  })
  const stream = new Duplex({
    read() {
      child.stdout.resume()
    },
    write(chunk, encoding, done) {
      child.stdin.write(chunk, encoding, done)
    },
    destroy(error, done) {
      child.stdin.destroy()
      child.kill()
      done(error)
    }
  })
  child.on('spawn', () => stream.emit('connect'))
  child.stdout.on('data', (chunk) => {
    if (!stream.push(chunk)) child.stdout.pause()
  })
  child.stdin.on('error', (error) => {
    if ((error as NodeJS.ErrnoException).code !== 'EPIPE') stream.destroy(error)
  })
  child.on('error', (error) => stream.destroy(error))
  child.on('close', (code) => {
    if (code === 3)
      stream.destroy(Object.assign(new Error('daemon not running'), { code: 'ENOENT' }))
    else stream.destroy()
  })
  return stream
}

/** One JSON-RPC round trip on a fresh connection to the daemon socket. */
export function callDaemon<T>(method: string, params?: unknown): Promise<HostlaneReply<T>> {
  const path = socketPath()
  const timeout = SLOW_METHODS.has(method) ? SLOW_CALL_TIMEOUT_MS : CALL_TIMEOUT_MS
  const request = JSON.stringify({
    jsonrpc: '2.0',
    id: nextId++,
    method,
    params: params ?? null,
    client: CLIENT
  })

  return new Promise((done) => {
    let buffer = ''
    let settled = false
    const socket = transport(path)
    socket.setEncoding('utf8')
    const settle = (reply: HostlaneReply<T>) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      socket.destroy()
      done(reply)
    }

    const timer = setTimeout(
      () => settle(failure('TIMEOUT', `守护进程在 ${timeout / 1000} 秒内没有响应`)),
      timeout
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
          settle({
            ok: false,
            error: { kind: data.kind, message, hint: data.hint, details: data.details }
          })
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

/**
 * Keeps one `events.subscribe` connection open and passes every event to `onEvent`.
 * Reconnects with backoff while the daemon is down; returns a function that stops it.
 */
export function subscribeEvents(onEvent: (event: HostlaneEvent) => void): () => void {
  let stopped = false
  let delay = RESUBSCRIBE_MIN_MS
  let timer: NodeJS.Timeout | undefined
  let socket: Duplex | undefined

  const retry = () => {
    socket = undefined
    if (stopped) return
    timer = setTimeout(connect, delay)
    delay = Math.min(delay * 2, RESUBSCRIBE_MAX_MS)
  }

  const connect = () => {
    let buffer = ''
    let acknowledged = false
    const current = transport(socketPath())
    current.setEncoding('utf8')
    socket = current
    current.on('connect', () =>
      current.write(
        `${JSON.stringify({ jsonrpc: '2.0', id: nextId++, method: 'events.subscribe', client: CLIENT })}\n`
      )
    )
    current.on('data', (chunk) => {
      buffer += chunk.toString('utf8')
      let end: number
      while ((end = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, end)
        buffer = buffer.slice(end + 1)
        let message: { method?: string; params?: Event; result?: unknown; error?: RpcError }
        try {
          message = JSON.parse(line)
        } catch {
          continue
        }
        if (!acknowledged) {
          if (message.error) {
            current.destroy()
            return
          }
          acknowledged = true
          delay = RESUBSCRIBE_MIN_MS
          onEvent({ type: 'subscribed' })
        } else if (message.method === 'event' && message.params) {
          onEvent(message.params)
        }
      }
    })
    current.on('error', () => {})
    current.on('close', retry)
  }

  connect()
  return () => {
    stopped = true
    clearTimeout(timer)
    socket?.destroy()
  }
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
    execFile(
      bin,
      ['daemon', 'start'],
      { timeout: START_TIMEOUT_MS, windowsHide: true },
      (error, _stdout, stderr) => {
        if (error) {
          done(failure('DAEMON_START_FAILED', stderr.trim() || error.message))
        } else {
          done({ ok: true, result: null })
        }
      }
    )
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
