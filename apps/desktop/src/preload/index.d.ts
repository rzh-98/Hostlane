import type { DesktopApi } from '../shared/ipc'

declare global {
  interface Window {
    /** Undefined when the renderer runs in a plain browser (e.g. `vite` preview). */
    desktop?: DesktopApi
  }
}
