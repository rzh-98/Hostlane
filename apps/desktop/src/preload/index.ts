import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron'
import { IpcChannel, type DesktopApi, type HostlaneEvent } from '../shared/ipc'

const api: DesktopApi = {
  platform: process.platform,
  versions: {
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node
  },
  setThemeSource: (source) => ipcRenderer.invoke(IpcChannel.setThemeSource, source),
  setTitleBarHeight: (height) => ipcRenderer.invoke(IpcChannel.setTitleBarHeight, height),
  hostlane: {
    call: (method, params) => ipcRenderer.invoke(IpcChannel.hostlaneCall, method, params),
    startDaemon: () => ipcRenderer.invoke(IpcChannel.hostlaneStartDaemon),
    onEvent: (listener) => {
      const handler = (_event: IpcRendererEvent, payload: HostlaneEvent) => listener(payload)
      ipcRenderer.on(IpcChannel.hostlaneEvent, handler)
      return () => {
        ipcRenderer.removeListener(IpcChannel.hostlaneEvent, handler)
      }
    }
  },
  onNavigate: (listener) => {
    const handler = (_event: IpcRendererEvent, page: string) => listener(page)
    ipcRenderer.on(IpcChannel.navigate, handler)
    return () => {
      ipcRenderer.removeListener(IpcChannel.navigate, handler)
    }
  }
}

contextBridge.exposeInMainWorld('desktop', api)
