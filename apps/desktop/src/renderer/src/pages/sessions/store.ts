import { callHostlane } from '@/app/daemon'
import { createStore } from '@/lib/store'
import type { HostlaneError, HostlaneEvent } from '@shared/ipc'
import type { ExecList } from '@shared/protocol/ExecList'
import type { ExecRecord } from '@shared/protocol/ExecRecord'
import type { OutputChunk } from '@shared/protocol/OutputChunk'
import type { OutputStream } from '@shared/protocol/OutputStream'

/** Commands loaded when the page opens. */
const HISTORY_LIMIT = 200
/** Bytes of each stream fetched for a finished command. */
export const TAIL_BYTES = 8 * 1024
/** Live output kept per command; older output is dropped from the view (not from disk). */
const LIVE_CAP = 64 * 1024
const FETCH_CONCURRENCY = 4

export interface OutputPiece {
  stream: OutputStream
  data: string
}

export interface ExecOutput {
  pieces: OutputPiece[]
  size: number
  /** Output before what is shown exists on disk (history tail, live cap, or opened mid-run). */
  clipped: boolean
  state: 'live' | 'loading' | 'loaded' | 'error'
}

interface SessionsState {
  execs: Record<string, ExecRecord>
  outputs: Record<string, ExecOutput>
  loaded: boolean
  error: HostlaneError | null
}

export const sessionsStore = createStore<SessionsState>(null, {
  execs: {},
  outputs: {},
  loaded: false,
  error: null
})

export async function loadSessions(): Promise<void> {
  const reply = await callHostlane<ExecList>('exec.list', { limit: HISTORY_LIMIT })
  if (!reply.ok) {
    sessionsStore.set({ error: reply.error, loaded: true })
    return
  }
  sessionsStore.set((state) => {
    const execs = { ...state.execs }
    const outputs = { ...state.outputs }
    for (const exec of reply.result.execs) {
      const known = execs[exec.id]
      if (!known || known.status === 'running') execs[exec.id] = exec
      if (exec.status === 'running' && !outputs[exec.id]) {
        outputs[exec.id] = {
          pieces: [],
          size: 0,
          clipped: exec.stdout_bytes + exec.stderr_bytes > 0,
          state: 'live'
        }
      }
    }
    return { execs, outputs, loaded: true, error: null }
  })
}

function append(output: ExecOutput, piece: OutputPiece): ExecOutput {
  const pieces = [...output.pieces]
  const last = pieces[pieces.length - 1]
  if (last && last.stream === piece.stream)
    pieces[pieces.length - 1] = { ...last, data: last.data + piece.data }
  else pieces.push(piece)
  let size = output.size + piece.data.length
  let clipped = output.clipped
  while (size > LIVE_CAP && pieces.length > 0) {
    const first = pieces[0]
    const excess = size - LIVE_CAP
    if (first.data.length <= excess) {
      pieces.shift()
      size -= first.data.length
    } else {
      pieces[0] = { ...first, data: first.data.slice(excess) }
      size -= excess
    }
    clipped = true
  }
  return { ...output, pieces, size, clipped }
}

let pending: HostlaneEvent[] = []
let frame = 0

function flush(): void {
  frame = 0
  const events = pending
  pending = []
  let reload = false
  sessionsStore.set((state) => {
    const execs = { ...state.execs }
    const outputs = { ...state.outputs }
    for (const event of events) {
      switch (event.type) {
        case 'exec_started':
          execs[event.exec.id] = event.exec
          outputs[event.exec.id] = { pieces: [], size: 0, clipped: false, state: 'live' }
          break
        case 'exec_output': {
          const output = outputs[event.id]
          if (output?.state === 'live') {
            outputs[event.id] = append(output, { stream: event.stream, data: event.data })
          }
          break
        }
        case 'exec_finished': {
          execs[event.exec.id] = event.exec
          const output = outputs[event.exec.id]
          if (output?.clipped) {
            // The live stream skipped data; show the stored tail instead (see ensureOutput).
            delete outputs[event.exec.id]
          } else if (output?.state === 'live') {
            outputs[event.exec.id] = { ...output, state: 'loaded' }
          }
          break
        }
        case 'subscribed':
        case 'lagged':
          reload = true
          break
      }
    }
    return { execs, outputs }
  })
  if (reload) void loadSessions()
}

/** Queues a daemon event; updates are applied once per animation frame. */
export function handleSessionEvent(event: HostlaneEvent): void {
  if (event.type === 'connection') return
  pending.push(event)
  if (!frame) frame = requestAnimationFrame(flush)
}

let queue: (() => Promise<void>)[] = []
let active = 0

function pump(): void {
  while (active < FETCH_CONCURRENCY && queue.length > 0) {
    const task = queue.shift()!
    active++
    void task().finally(() => {
      active--
      pump()
    })
  }
}

function runQueued(task: () => Promise<void>): void {
  queue.push(task)
  pump()
}

async function readTail(id: string, stream: OutputStream, bytes: number) {
  if (bytes === 0) return { data: '', ok: true }
  const reply = await callHostlane<OutputChunk>('output.read', {
    output_id: id,
    stream,
    offset: bytes > TAIL_BYTES ? -TAIL_BYTES : 0,
    limit: TAIL_BYTES
  })
  return reply.ok ? { data: reply.result.data, ok: true } : { data: '', ok: false }
}

/** Fetches the tail of a finished command's output the first time it is shown. */
export function ensureOutput(exec: ExecRecord): void {
  const { outputs } = sessionsStore.get()
  if (outputs[exec.id] || exec.status === 'running') return
  const empty = exec.stdout_bytes + exec.stderr_bytes === 0
  sessionsStore.set((state) => ({
    outputs: {
      ...state.outputs,
      [exec.id]: { pieces: [], size: 0, clipped: false, state: empty ? 'loaded' : 'loading' }
    }
  }))
  if (empty) return
  runQueued(async () => {
    const [stdout, stderr] = await Promise.all([
      readTail(exec.id, 'stdout', exec.stdout_bytes),
      readTail(exec.id, 'stderr', exec.stderr_bytes)
    ])
    const pieces: OutputPiece[] = []
    if (stdout.data) pieces.push({ stream: 'stdout', data: stdout.data })
    if (stderr.data) pieces.push({ stream: 'stderr', data: stderr.data })
    sessionsStore.set((state) => ({
      outputs: {
        ...state.outputs,
        [exec.id]: {
          pieces,
          size: stdout.data.length + stderr.data.length,
          clipped: exec.stdout_bytes > TAIL_BYTES || exec.stderr_bytes > TAIL_BYTES,
          state: stdout.ok && stderr.ok ? 'loaded' : 'error'
        }
      }
    }))
  })
}

export function resetSessions(): void {
  pending = []
  queue = []
  if (frame) cancelAnimationFrame(frame)
  frame = 0
  sessionsStore.reset()
}
