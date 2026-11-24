import { useEffect } from 'react'
import { createStore } from '@/lib/store'
import type { HostlaneError, SetupStatus } from '@shared/ipc'

interface SetupState {
  status: SetupStatus | null
  /** An install or link action is running. */
  busy: boolean
}

/** The CLI link and the daemon service, shown on the integrations and settings pages. */
export const setupStore = createStore<SetupState>(null, { status: null, busy: false })

export async function refreshSetup(): Promise<void> {
  if (!window.desktop) return
  setupStore.set({ status: await window.desktop.setup.status() })
}

/** Loads the status when a page that shows it mounts. */
export function useSetup(): SetupState {
  useEffect(() => {
    void refreshSetup()
  }, [])
  return setupStore.use()
}

/** Runs a setup action, then reloads the status. Returns the error, if any. */
export async function runSetup(
  action: 'linkCli' | 'installService' | 'uninstallService'
): Promise<HostlaneError | null> {
  if (!window.desktop) return { kind: 'UNAVAILABLE', message: '需要在 Hostlane 桌面端中运行' }
  setupStore.set({ busy: true })
  try {
    const reply = await window.desktop.setup[action]()
    return reply.ok ? null : reply.error
  } finally {
    await refreshSetup()
    setupStore.set({ busy: false })
  }
}
