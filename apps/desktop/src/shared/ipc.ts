import type { ErrorKind } from './protocol/ErrorKind'
import type { Event } from './protocol/Event'

import type methods from './renderer-methods.json'

export type ThemeSource = 'system' | 'light' | 'dark'

/** Daemon methods the renderer may call. The main process rejects anything else. */
export type RendererMethod = keyof typeof methods

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

/** `hostlane --json daemon service`. */
export interface ServiceStatus {
  supported: boolean
  manager: 'launchd' | 'systemd' | 'scheduler' | null
  installed: boolean
  path: string | null
  /** The `hostlane` binary the installed service runs. */
  binary: string | null
}

/** How agents reach Hostlane from this machine: the CLI on PATH and the daemon service. */
export interface SetupStatus {
  /** The `hostlane` binary this app uses. */
  binary: string
  binaryExists: boolean
  serviceOutdated: boolean
  /** Command for MCP configs: the PATH link when it points at `binary`, else `binary`. */
  command: string
  /** HOSTLANE_* variables this app runs with, repeated in MCP configs. */
  env: Record<string, string>
  cli: {
    supported: boolean
    linkPath: string
    /** Where `linkPath` points now, or null if it does not exist. */
    linkTarget: string | null
    linked: boolean
    /** Whether the link's directory is on the login shell's PATH. */
    dirOnPath: boolean
  }
  /** Null when the binary is missing or the status could not be read. */
  service: ServiceStatus | null
}

/** The API exposed to the renderer as `window.desktop`. Keep it small and explicit. */
export interface DesktopApi {
  /** Node's `process.platform`: 'darwin' | 'win32' | 'linux' | ... */
  platform: string
  openExternal: (url: string) => Promise<void>
  setThemeSource: (source: ThemeSource) => Promise<void>
  hostlane: {
    promptSecret: (
      host: string,
      kind: 'password' | 'passphrase',
      key?: string
    ) => Promise<HostlaneReply<boolean>>
    chooseKey: () => Promise<string | null>
    revealTrustRecord: (host: string) => Promise<boolean>
    exportAudit: (
      format: 'jsonl' | 'csv',
      scope: import('./protocol/AuditListParams').AuditListParams
    ) => Promise<HostlaneReply<string | null>>
    call: <T>(method: RendererMethod, params?: unknown) => Promise<HostlaneReply<T>>
    /** Starts the background daemon if needed (`hostlane daemon start`). */
    startDaemon: () => Promise<HostlaneReply<null>>
    /** Listens to daemon events; returns the unsubscribe function. */
    onEvent: (listener: (event: HostlaneEvent) => void) => () => void
  }
  setup: {
    configureMcp: (
      client: 'codex' | 'claude' | 'cursor',
      token?: string
    ) => Promise<HostlaneReply<string | null>>
    status: () => Promise<SetupStatus>
    /** Links `~/.local/bin/hostlane` to this app's binary. */
    linkCli: () => Promise<HostlaneReply<SetupStatus>>
    /** Registers the daemon as a login service (launchd / systemd --user). */
    installService: () => Promise<HostlaneReply<unknown>>
    uninstallService: () => Promise<HostlaneReply<unknown>>
  }
  /** Page requests from the main process, e.g. after clicking an approval notification. */
  onNavigate: (listener: (page: string) => void) => () => void
}
