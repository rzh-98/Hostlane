import { app, BrowserWindow, Notification } from 'electron'
import { IpcChannel, type HostlaneEvent } from '../shared/ipc'
import type { Approval } from '../shared/protocol/Approval'
import type { PolicyStatus } from '../shared/protocol/PolicyStatus'
import { callDaemon } from './daemon'
import { createMainWindow } from './window'

const RISK_LABELS = { L0: '只读', L1: '用户态修改', L2: '系统变更', L3: '破坏性操作' } as const

/** Held so the click handlers survive garbage collection; removed once decided. */
const notifications = new Map<string, Notification>()

/** Brings the window forward on `page`, creating it if every window was closed. */
export function showPage(page: string): void {
  const [win] = BrowserWindow.getAllWindows()
  if (!win) {
    createMainWindow(page)
    return
  }
  if (win.isMinimized()) win.restore()
  win.show()
  win.focus()
  win.webContents.send(IpcChannel.navigate, page)
}

function notify(approval: Approval): void {
  if (!Notification.isSupported() || BrowserWindow.getFocusedWindow()) return
  const where = [approval.host, approval.env].filter(Boolean).join(' · ')
  const notification = new Notification({
    title: `${approval.client.name} 请求审批`,
    body: `${where ? `${where} · ` : ''}${approval.risk} ${RISK_LABELS[approval.risk]}\n${approval.summary}`
  })
  notification.on('click', () => showPage('approvals'))
  notification.on('close', () => notifications.delete(approval.id))
  notifications.set(approval.id, notification)
  notification.show()
}

function setBadge(pending: number): void {
  app.setBadgeCount(pending)
}

/**
 * Approval notifications and the dock badge. The daemon skips its own system
 * notification while the desktop app is subscribed, so this must handle them.
 */
export function handlePolicyEvent(event: HostlaneEvent): void {
  switch (event.type) {
    case 'approval_requested':
      notify(event.approval)
      break
    case 'approval_updated':
      if (event.approval.state !== 'pending') {
        notifications.get(event.approval.id)?.close()
        notifications.delete(event.approval.id)
      }
      break
    case 'policy':
      setBadge(event.status.pending_approvals)
      break
    case 'subscribed':
      void callDaemon<PolicyStatus>('policy.status').then((reply) => {
        if (reply.ok) setBadge(reply.result.pending_approvals)
      })
      break
  }
}
