import type { ErrorKind } from './protocol/ErrorKind'

/** IPC channel names and payload types shared by main, preload and renderer. */
export const IpcChannel = {
  setThemeSource: 'app:set-theme-source',
  setTitleBarHeight: 'window:set-title-bar-height',
  hostlaneCall: 'hostlane:call',
  hostlaneStartDaemon: 'hostlane:start-daemon'
} as const

export type ThemeSource = 'system' | 'light' | 'dark'

/** Daemon methods the renderer may call. The main process rejects anything else. */
export const RENDERER_METHODS = ['daemon.status', 'hosts.list'] as const
export type RendererMethod = (typeof RENDERER_METHODS)[number]

/** Daemon error kinds plus failures that happen before a request reaches the daemon. */
export type HostlaneErrorKind =
  | ErrorKind
  | 'DAEMON_NOT_RUNNING'
  | 'DAEMON_BINARY_MISSING'
  | 'DAEMON_START_FAILED'
  | 'TIMEOUT'
  | 'IO'
  | 'UNAVAILABLE'

export interface HostlaneError {
  kind: HostlaneErrorKind
  message: string
  hint?: string
}

/** Errors are returned, not thrown, so their structure survives the IPC boundary. */
export type HostlaneReply<T> = { ok: true; result: T } | { ok: false; error: HostlaneError }

/** The API exposed to the renderer as `window.desktop`. Keep it small and explicit. */
export interface DesktopApi {
  /** Node's `process.platform`: 'darwin' | 'win32' | 'linux' | ... */
  platform: string
  versions: { electron: string; chrome: string; node: string }
  setThemeSource: (source: ThemeSource) => Promise<void>
  /** Keeps macOS traffic lights vertically centred in the toolbar. */
  setTitleBarHeight: (height: number) => Promise<void>
  hostlane: {
    call: <T>(method: RendererMethod, params?: unknown) => Promise<HostlaneReply<T>>
    /** Starts the background daemon if needed (`hostlane daemon start`). */
    startDaemon: () => Promise<HostlaneReply<null>>
  }
}
