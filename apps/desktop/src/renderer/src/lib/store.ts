import { useSyncExternalStore } from 'react'
import { readJson, writeJson } from './storage'

export interface Store<T extends object> {
  get: () => T
  set: (patch: Partial<T> | ((state: T) => Partial<T>)) => void
  reset: () => void
  subscribe: (listener: () => void) => () => void
  /** React hook returning the current state. */
  use: () => T
}

/**
 * Tiny global store for state that outlives a page or is shared between pages.
 * Pass a localStorage key to persist it, or null to keep it for the session only.
 *
 *   export const prefs = createStore('my-feature.prefs', { compact: false })
 *   const { compact } = prefs.use()          // in a component; re-renders on change
 *   prefs.set({ compact: true })             // anywhere
 *   prefs.set((s) => ({ count: s.count + 1 }))
 *
 * State is replaced shallowly on every `set`; keep values JSON-serialisable when persisted.
 */
export function createStore<T extends object>(key: string | null, initial: T): Store<T> {
  let state: T = key ? readJson(key, initial) : initial
  const listeners = new Set<() => void>()

  const store: Store<T> = {
    get: () => state,
    set: (patch) => {
      state = { ...state, ...(typeof patch === 'function' ? patch(state) : patch) }
      if (key) writeJson(key, state)
      for (const listener of listeners) listener()
    },
    reset: () => store.set(initial),
    subscribe: (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    use: () => useSyncExternalStore(store.subscribe, store.get)
  }
  return store
}
