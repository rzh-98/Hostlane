/** `darwin`, `win32`, `linux` — or `web` when running outside Electron. */
export const platform: string = window.desktop?.platform ?? 'web'
export const isMac = platform === 'darwin' || (platform === 'web' && /Mac/.test(navigator.platform))

/** Label for the primary modifier key, e.g. in shortcut hints. */
export const modKey = isMac ? '⌘' : 'Ctrl+'

/** True when the platform's primary modifier (⌘ on macOS, Ctrl elsewhere) is held. */
export function isModKey(event: KeyboardEvent | React.KeyboardEvent): boolean {
  return isMac ? event.metaKey : event.ctrlKey
}
