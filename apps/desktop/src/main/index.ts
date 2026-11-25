import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  nativeTheme,
  shell,
  type IpcMainInvokeEvent
} from 'electron'
import { homedir } from 'node:os'
import { isAbsolute, join } from 'node:path'
import type { ConnectionsList } from '../shared/protocol/ConnectionsList'
import { configureMcp, exportAudit } from './actions'
import { promptSecret } from './credentials'
import { IpcChannel, RENDERER_METHODS, type ThemeSource } from '../shared/ipc'
import { handlePolicyEvent } from './approvals'
import { callDaemon, ensureDaemon, startDaemon, subscribeEvents } from './daemon'
import { installService, linkCli, setupStatus, uninstallService } from './setup'
import { createMainWindow, trafficLightPosition, windowBackground } from './window'

const THEME_SOURCES: ThemeSource[] = ['system', 'light', 'dark']

function registerIpc(): void {
  function owner(event: IpcMainInvokeEvent): BrowserWindow {
    const win = BrowserWindow.fromWebContents(event.sender)
    if (!win || !event.senderFrame || event.senderFrame !== event.sender.mainFrame)
      throw new Error('需要在 Hostlane 主窗口中操作')
    return win
  }
  ipcMain.handle(IpcChannel.promptSecret, async (event, host, kind, key) => {
    const win = owner(event)
    const controller = new AbortController()
    const abort = () => controller.abort()
    win.once('closed', abort)
    try {
      return await promptSecret(host, kind, key, controller.signal)
    } finally {
      win.off('closed', abort)
    }
  })
  ipcMain.handle(IpcChannel.chooseKey, async (event) => {
    const result = await dialog.showOpenDialog(owner(event), {
      title: '选择 SSH 私钥文件',
      defaultPath: join(homedir(), '.ssh'),
      properties: ['openFile', 'showHiddenFiles']
    })
    return result.canceled ? null : (result.filePaths[0] ?? null)
  })
  ipcMain.handle(IpcChannel.revealTrustRecord, async (event, host) => {
    owner(event)
    if (typeof host !== 'string') return false
    const connections = await callDaemon<ConnectionsList>('connections.list')
    if (!connections.ok) return false
    const error = connections.result.connections.find((item) => item.host === host)?.error?.data
    if (error?.kind !== 'HOST_KEY_CHANGED') return false
    const location = (error.details as { recorded_at?: unknown } | undefined)?.recorded_at
    const file = typeof location === 'string' ? location.match(/^(.+):\d+$/)?.[1] : undefined
    if (!file || !isAbsolute(file) || /^[/\\]{2}/.test(file)) return false
    shell.showItemInFolder(file)
    return true
  })
  ipcMain.handle(IpcChannel.configureMcp, (event, client, token) =>
    configureMcp(owner(event), client, token)
  )
  ipcMain.handle(IpcChannel.exportAudit, (event, format, scope) =>
    exportAudit(owner(event), format, scope)
  )
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
  ipcMain.handle(IpcChannel.setupStatus, () => setupStatus())
  ipcMain.handle(IpcChannel.setupLinkCli, () => linkCli())
  ipcMain.handle(IpcChannel.setupInstallService, () => installService())
  ipcMain.handle(IpcChannel.setupUninstallService, () => uninstallService())

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
