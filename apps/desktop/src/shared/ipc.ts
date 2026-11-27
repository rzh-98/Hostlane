import type { ErrorKind } from './protocol/ErrorKind'
import type { Event } from './protocol/Event'

/** IPC channel names and payload types shared by main, preload and renderer. */
export const IpcChannel = {
  setThemeSource: 'app:set-theme-source',
  setTitleBarHeight: 'window:set-title-bar-height',
  hostlaneCall: 'hostlane:call',
  hostlaneStartDaemon: 'hostlane:start-daemon',
  hostlaneEvent: 'hostlane:event',
  navigate: 'app:navigate',
  setupStatus: 'setup:status',
  setupLinkCli: 'setup:link-cli',
  promptSecret: 'hostlane:prompt-secret',
  chooseKey: 'hostlane:choose-key',
  revealTrustRecord: 'hostlane:reveal-trust-record',
  configureMcp: 'setup:configure-mcp',
  exportAudit: 'hostlane:export-audit',
  setupInstallService: 'setup:install-service',
  setupUninstallService: 'setup:uninstall-service'
} as const

export type ThemeSource = 'system' | 'light' | 'dark'

/** Daemon methods the renderer may call. The main process rejects anything else. */
export const RENDERER_METHODS = [
  'daemon.status',
  'hosts.list',
  'hosts.connect',
  'hosts.disconnect',
  'hosts.trust',
  'hosts.add',
  'hosts.update',
  'hosts.remove',
  'secrets.list',
  'secrets.delete',
  'connections.list',
  'exec.list',
  'output.read',
  'files.change',
  'files.rollback_plan',
  'files.revert',
  'ports.list',
  'ports.inspect',
  'ports.check',
  'ports.find_free',
  'ports.annotate',
  'services.logs',
  'services.start',
  'services.stop',
  'services.restart',
  'processes.kill',
  'jobs.list',
  'jobs.status',
  'jobs.logs',
  'jobs.cancel',
  'clients.list',
  'clients.create',
  'clients.revoke',
  'clients.configure',
  'tunnels.list',
  'tunnels.open',
  'tunnels.close',
  'tunnels.start',
  'tunnels.update',
  'tunnels.delete',
  'audit.list',
  'hosts.label',
  'hosts.profile',
  'hosts.export',
  'hosts.note',
  'web.report',
  'health.list',
  'reachability.list',
  'reachability.set',
  'health.set',
  'reverse.open',
  'reverse.close',
  'reverse.list',
  'approvals.list',
  'approvals.get',
  'approvals.resolve',
  'policy.status',
  'policy.stop',
  'policy.resume',
  'policy.check',
  'policy.rules.list',
  'policy.rules.add',
  'policy.rules.delete'
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
  versions: { electron: string; chrome: string; node: string }
  setThemeSource: (source: ThemeSource) => Promise<void>
  /** Keeps macOS traffic lights vertically centred in the toolbar. */
  setTitleBarHeight: (height: number) => Promise<void>
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
