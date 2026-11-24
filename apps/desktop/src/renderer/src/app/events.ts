import { useEffect, useRef } from 'react'
import type { HostlaneEvent } from '@shared/ipc'

type Listener = (event: HostlaneEvent) => void

const listeners = new Set<Listener>()
let detach: (() => void) | null = null

/** Listens to daemon events forwarded by the main process; returns the unsubscribe function. */
export function onHostlaneEvent(listener: Listener): () => void {
  listeners.add(listener)
  if (!detach && window.desktop) {
    detach = window.desktop.hostlane.onEvent((event) => {
      for (const current of listeners) current(event)
    })
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && detach) {
      detach()
      detach = null
    }
  }
}

/** Calls `handler` for every daemon event while the component is mounted. */
export function useHostlaneEvent(handler: Listener): void {
  const latest = useRef(handler)
  useEffect(() => {
    latest.current = handler
  })
  useEffect(() => onHostlaneEvent((event) => latest.current(event)), [])
}
