import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { DesktopApi } from '@shared/ipc'

function subscribe<T>(name: string, listener: (payload: T) => void) {
  const ready = listen<T>(name, ({ payload }) => listener(payload))
  return () => {
    void ready.then((unlisten) => unlisten())
  }
}
const api: DesktopApi = {
  platform: import.meta.env.VITE_DESKTOP_PLATFORM,
  openExternal: (url) => invoke('open_external', { url }),
  setThemeSource: (source) => getCurrentWindow().setTheme(source === 'system' ? null : source),
  hostlane: {
    call: (method, params) => invoke('hostlane_call', { method, params }),
    startDaemon: () => invoke('start_daemon'),
    promptSecret: (host, kind, key) => invoke('prompt_secret', { host, kind, key }),
    chooseKey: () => invoke('choose_key'),
    revealTrustRecord: (host) => invoke('reveal_trust_record', { host }),
    exportAudit: (format, scope) => invoke('export_audit', { format, scope }),
    onEvent: (listener) => subscribe('hostlane-event', listener)
  },
  setup: {
    configureMcp: (client, token) => invoke('configure_mcp', { client, token }),
    status: () => invoke('setup_status'),
    linkCli: () => invoke('link_cli'),
    installService: () => invoke('install_service'),
    uninstallService: () => invoke('uninstall_service')
  },
  onNavigate: (listener) => subscribe('navigate', listener)
}
Object.defineProperty(window, 'desktop', { value: api })
document.addEventListener('mousedown', (event) => {
  const target = event.target
  if (
    api.platform === 'darwin' &&
    event.button === 0 &&
    target instanceof Element &&
    target.closest('[data-drag-region]') &&
    !target.closest('button,input,textarea,select,a,[role],[tabindex]')
  )
    void getCurrentWindow().startDragging()
})
void import('./main')
