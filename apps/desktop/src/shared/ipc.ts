import type { ErrorKind } from './protocol/ErrorKind'
import type { Event } from './protocol/Event'

/** IPC channel names and payload types shared by main, preload and renderer. */
export const IpcChannel = {
  setThemeSource: 'app:set-theme-source',
  setTitleBarHeight: 'window:set-title-bar-height',
  hostlaneCall: 'hostlane:call',
  hostlaneStartDaemon: 'hostlane:start-daemon',
  hostlaneEvent: 'hostlane:event'
} as const

export type ThemeSource = 'system' | 'light' | 'dark'

/** Daemon methods the renderer may call. The main process rejects anything else. */
export const RENDERER_METHODS = [
  'daemon.status',
  'hosts.list',
  'hosts.connect',
  'hosts.disconnect',
  'hosts.trust',
  'connections.list',
  'exec.list',
  'output.read',
  'files.change',
  'files.revert',
  'ports.list',
  'ports.inspect',
  'ports.check',
  'ports.find_free',
  'ports.annotate',
  'audit.list'
] as const
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
  /** Structured context for some kinds, e.g. the host key for HOST_KEY_UNKNOWN. */
  details?: Record<string, unknown>
}

/** Errors are returned, not thrown, so their structure survives the IPC boundary. */
export type HostlaneReply<T> = { ok: true; result: T } | { ok: false; error: HostlaneError }

/**
 * Daemon events forwarded by the main process, plus `subscribed`, sent each time the
 * subscription is (re)established: events may have been missed, so reload state.
 */
export type HostlaneEvent = Event | { type: 'subscribed' }

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
    /** Listens to daemon events; returns the unsubscribe function. */
    onEvent: (listener: (event: HostlaneEvent) => void) => () => void
  }
}
