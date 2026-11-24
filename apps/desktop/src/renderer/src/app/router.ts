import { useSyncExternalStore } from 'react'

/** Minimal hash router: `#/settings` → "settings". Enough for a desktop app's page switcher. */
function currentRoute(): string | null {
  const id = window.location.hash.replace(/^#\/?/, '').split(/[/?]/)[0]
  return id || null
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener('hashchange', onChange)
  return () => window.removeEventListener('hashchange', onChange)
}

export function useRoute(): string | null {
  return useSyncExternalStore(subscribe, currentRoute)
}

export function navigate(id: string): void {
  window.location.hash = `/${id}`
}
