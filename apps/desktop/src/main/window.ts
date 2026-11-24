import { app, BrowserWindow, nativeTheme, shell } from 'electron'
import { join } from 'node:path'
import icon from '../../resources/icon.png?asset'

const isMac = process.platform === 'darwin'

/** Shown before the renderer paints. Keep roughly in sync with `--color-bg` in tokens.css. */
const BACKGROUND = { light: '#fafafa', dark: '#1c1c1f' }

/** macOS traffic light size; used to centre them inside the toolbar. */
const TRAFFIC_LIGHT = { x: 16, height: 14 }
const DEFAULT_TITLE_BAR_HEIGHT = 44

export function windowBackground(): string {
  return nativeTheme.shouldUseDarkColors ? BACKGROUND.dark : BACKGROUND.light
}

export function trafficLightPosition(titleBarHeight: number): { x: number; y: number } {
  return { x: TRAFFIC_LIGHT.x, y: Math.round((titleBarHeight - TRAFFIC_LIGHT.height) / 2) }
}

/** `page` opens a route other than the default, e.g. `approvals`. */
export function createMainWindow(page?: string): BrowserWindow {
  const win = new BrowserWindow({
    width: 1180,
    height: 760,
    minWidth: 720,
    minHeight: 480,
    show: false,
    backgroundColor: windowBackground(),
    ...(isMac
      ? {
          titleBarStyle: 'hiddenInset' as const,
          trafficLightPosition: trafficLightPosition(DEFAULT_TITLE_BAR_HEIGHT)
        }
      : { autoHideMenuBar: true }),
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      backgroundThrottling: false
    }
  })

  win.once('ready-to-show', () => win.show())

  // Windows / Linux: Electron's default File/Edit/View menu doesn't belong in a tool UI.
  // In development it stays reachable with Alt (DevTools, reload); packaged builds drop it.
  if (!isMac && app.isPackaged) win.removeMenu()

  // Open external links in the user's browser instead of a new Electron window.
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//.test(url)) void shell.openExternal(url)
    return { action: 'deny' }
  })
  win.webContents.on('will-navigate', (event, url) => {
    if (url !== win.webContents.getURL()) event.preventDefault()
  })

  const devServerUrl = process.env['ELECTRON_RENDERER_URL']
  const hash = page ? `/${page}` : undefined
  if (devServerUrl) {
    void win.loadURL(hash ? `${devServerUrl}#${hash}` : devServerUrl)
  } else {
    void win.loadFile(join(__dirname, '../renderer/index.html'), { hash })
  }

  return win
}
