import { app, BrowserWindow, ipcMain, nativeTheme } from 'electron'
import { IpcChannel, RENDERER_METHODS, type ThemeSource } from '../shared/ipc'
import { handlePolicyEvent } from './approvals'
import { callDaemon, ensureDaemon, startDaemon, subscribeEvents } from './daemon'
import { createMainWindow, trafficLightPosition, windowBackground } from './window'

const THEME_SOURCES: ThemeSource[] = ['system', 'light', 'dark']

function registerIpc(): void {
  ipcMain.handle(IpcChannel.setThemeSource, (_event, source: unknown) => {
    if (THEME_SOURCES.includes(source as ThemeSource)) {
      nativeTheme.themeSource = source as ThemeSource
    }
  })

  ipcMain.handle(IpcChannel.setTitleBarHeight, (event, height: unknown) => {
    const win = BrowserWindow.fromWebContents(event.sender)
    if (process.platform !== 'darwin' || !win || typeof height !== 'number') return
    if (height < 24 || height > 96) return
    win.setWindowButtonPosition(trafficLightPosition(height))
  })

  ipcMain.handle(IpcChannel.hostlaneCall, (_event, method: unknown, params: unknown) => {
    if (!RENDERER_METHODS.includes(method as (typeof RENDERER_METHODS)[number])) {
      return {
        ok: false,
        error: { kind: 'METHOD_NOT_FOUND', message: `渲染进程不能调用 ${String(method)}` }
      }
    }
    return callDaemon(method as string, params)
  })

  ipcMain.handle(IpcChannel.hostlaneStartDaemon, () => startDaemon())

  nativeTheme.on('updated', () => {
    for (const win of BrowserWindow.getAllWindows()) win.setBackgroundColor(windowBackground())
  })
}

if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app.on('second-instance', () => {
    const [win] = BrowserWindow.getAllWindows()
    if (!win) return
    if (win.isMinimized()) win.restore()
    win.focus()
  })

  app.whenReady().then(() => {
    registerIpc()
    createMainWindow()
    void ensureDaemon()
    const unsubscribe = subscribeEvents((event) => {
      handlePolicyEvent(event)
      for (const win of BrowserWindow.getAllWindows()) {
        if (!win.isDestroyed()) win.webContents.send(IpcChannel.hostlaneEvent, event)
      }
    })
    app.on('will-quit', unsubscribe)

    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createMainWindow()
    })
  })

  app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') app.quit()
  })
}
