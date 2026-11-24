import { useEffect } from 'react'
import { createStore } from '@/lib/store'
import type { Connection } from '@shared/protocol/Connection'
import type { ConnectionsList } from '@shared/protocol/ConnectionsList'
import { callHostlane, daemonStore } from './daemon'
import { useHostlaneEvent } from './events'

/** SSH connection state per host, kept current by `connection` events. */
export const connectionsStore = createStore<{ byHost: Record<string, Connection> }>(null, {
  byHost: {}
})

export async function loadConnections(): Promise<void> {
  const reply = await callHostlane<ConnectionsList>('connections.list')
  if (!reply.ok) return
  connectionsStore.set({
    byHost: Object.fromEntries(reply.result.connections.map((c) => [c.host, c]))
  })
}

export function setConnection(connection: Connection): void {
  connectionsStore.set((state) => ({
    byHost: { ...state.byHost, [connection.host]: connection }
  }))
}

/** Mount once, near the root. */
export function useConnectionsSync(): void {
  const { phase } = daemonStore.use()
  useEffect(() => {
    if (phase === 'online') void loadConnections()
    else if (phase === 'offline') connectionsStore.reset()
  }, [phase])
  useHostlaneEvent((event) => {
    if (event.type === 'connection') setConnection(event.connection)
    else if (event.type === 'subscribed' || event.type === 'lagged') void loadConnections()
  })
}

export type ConnectionTone = 'neutral' | 'success' | 'warning' | 'danger'

/** Short label and color for a host's connection, including host key problems. */
export function describeConnection(connection: Connection | undefined): {
  label: string
  tone: ConnectionTone
} {
  switch (connection?.state) {
    case 'connected':
      return { label: '已连接', tone: 'success' }
    case 'connecting':
      return { label: '连接中', tone: 'warning' }
    case 'failed':
      switch (connection.error?.data.kind) {
        case 'HOST_KEY_UNKNOWN':
          return { label: '待信任', tone: 'warning' }
        case 'HOST_KEY_CHANGED':
          return { label: '指纹已变化', tone: 'danger' }
        case 'AUTH_FAILED':
          return { label: '认证失败', tone: 'danger' }
        default:
          return { label: '连接失败', tone: 'danger' }
      }
    default:
      return { label: '未连接', tone: 'neutral' }
  }
}
