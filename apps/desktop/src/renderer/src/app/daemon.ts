import { useEffect } from 'react'
import { createStore } from '@/lib/store'
import type { HostlaneError, HostlaneReply, RendererMethod } from '@shared/ipc'
import type { DaemonStatus } from '@shared/protocol/DaemonStatus'

const POLL_INTERVAL_MS = 5_000

/** Incremented on every status request and start attempt; older replies are dropped. */
let generation = 0

export type DaemonPhase = 'checking' | 'online' | 'offline' | 'starting'

interface DaemonState {
  phase: DaemonPhase
  status: DaemonStatus | null
  error: HostlaneError | null
}

/** Connection state of the Hostlane daemon, shared by the sidebar and every page. */
export const daemonStore = createStore<DaemonState>(null, {
  phase: 'checking',
  status: null,
  error: null
})

/** Calls the daemon through the main process. Never throws. */
export function callHostlane<T>(
  method: RendererMethod,
  params?: unknown
): Promise<HostlaneReply<T>> {
  if (!window.desktop) {
    return Promise.resolve({
      ok: false,
      error: { kind: 'UNAVAILABLE', message: '需要在 Hostlane 桌面端中运行' }
    })
  }
  return window.desktop.hostlane.call<T>(method, params)
}

export async function refreshDaemon(): Promise<void> {
  const request = ++generation
  const reply = await callHostlane<DaemonStatus>('daemon.status')
  if (request !== generation || daemonStore.get().phase === 'starting') return
  daemonStore.set(
    reply.ok
      ? { phase: 'online', status: reply.result, error: null }
      : { phase: 'offline', status: null, error: reply.error }
  )
}

export async function startDaemon(): Promise<void> {
  if (!window.desktop) return
  generation++
  daemonStore.set({ phase: 'starting', error: null })
  const reply = await window.desktop.hostlane.startDaemon()
  daemonStore.set({ phase: 'checking' })
  if (!reply.ok) {
    daemonStore.set({ phase: 'offline', error: reply.error })
    return
  }
  await refreshDaemon()
}

/** Keeps `daemonStore` fresh. Mount once, near the root. */
export function useDaemonPolling(): void {
  useEffect(() => {
    void refreshDaemon()
    const timer = window.setInterval(() => void refreshDaemon(), POLL_INTERVAL_MS)
    return () => window.clearInterval(timer)
  }, [])
}
