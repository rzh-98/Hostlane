import { execFile, spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { open, rename, rm } from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { dialog, type BrowserWindow } from 'electron'
import type { HostlaneReply } from '../shared/ipc'
import { binaryPath } from './daemon'

let configuring = false
export async function configureMcp(
  win: BrowserWindow,
  client: unknown,
  token: unknown
): Promise<HostlaneReply<string | null>> {
  if (
    (client !== 'codex' && client !== 'claude' && client !== 'cursor') ||
    (token !== undefined && (typeof token !== 'string' || !/^hlt_[a-f0-9]{64}$/.test(token)))
  )
    return { ok: false, error: { kind: 'INVALID_PARAMS', message: '无效的客户端或令牌' } }
  if (configuring) return { ok: false, error: { kind: 'IO', message: '已有配置操作正在进行。' } }
  configuring = true
  try {
    const home = homedir()
    const file =
      client === 'codex'
        ? join(process.env.CODEX_HOME || join(home, '.codex'), 'config.toml')
        : client === 'cursor'
          ? join(home, '.cursor', 'mcp.json')
          : join(process.env.CLAUDE_CONFIG_DIR || home, '.claude.json')
    const { response } = await dialog.showMessageBox(win, {
      type: 'question',
      buttons: ['取消', '写入配置'],
      defaultId: 0,
      cancelId: 0,
      message: `将 Hostlane 接入 ${client}？`,
      detail: `请先关闭目标客户端。仅添加或更新 hostlane 条目，保留其他设置。${token ? '令牌将保存在该客户端的本机配置文件中。' : ''}\n\n${file}`
    })
    if (response !== 1) return { ok: true, result: null }
    return await new Promise<HostlaneReply<string | null>>((resolve) => {
      const child = execFile(
        binaryPath(),
        ['configure-mcp', client, file],
        { windowsHide: true, timeout: 15000 },
        (error, _stdout, stderr) =>
          resolve(
            error
              ? {
                  ok: false,
                  error: { kind: 'IO', message: stderr.trim() || '写入失败，请检查文件权限。' }
                }
              : { ok: true, result: file }
          )
      )
      child.stdin?.on('error', () => {})
      child.stdin?.end(token ?? '')
    })
  } finally {
    configuring = false
  }
}

export async function exportAudit(
  win: BrowserWindow,
  format: unknown,
  scope: unknown
): Promise<HostlaneReply<string | null>> {
  if ((format !== 'csv' && format !== 'jsonl') || !scope || typeof scope !== 'object')
    return { ok: false, error: { kind: 'INVALID_PARAMS', message: '无效的导出选项' } }
  const args = ['audit-export', '--format', format]
  for (const [name, value] of Object.entries(scope)) {
    if (value === undefined) continue
    if (
      ['client', 'task', 'session', 'host'].includes(name) &&
      typeof value === 'string' &&
      value.length <= 4096 &&
      !value.includes('\0')
    )
      args.push(`--${name}=${value}`)
    else if (name === 'min_risk' && ['L0', 'L1', 'L2', 'L3'].includes(value))
      args.push(`--min-risk=${value.toLowerCase()}`)
    else if (name === 'since' && Number.isSafeInteger(value) && value >= 0)
      args.push(`--since=${value}`)
    else return { ok: false, error: { kind: 'INVALID_PARAMS', message: '无效的审计筛选条件' } }
  }
  const { filePath, canceled } = await dialog.showSaveDialog(win, {
    title: '导出符合筛选条件的全部审计记录',
    defaultPath: `hostlane-audit.${format}`,
    filters: [{ name: format.toUpperCase(), extensions: [format] }]
  })
  if (canceled || !filePath) return { ok: true, result: null }
  const temp = join(dirname(filePath), `.hostlane-${randomUUID()}.tmp`)
  try {
    const file = await open(temp, 'wx', 0o600)
    try {
      await new Promise<void>((resolve, reject) => {
        const child = spawn(binaryPath(), args, {
          windowsHide: true,
          stdio: ['ignore', file.fd, 'ignore'],
          env: { ...process.env, HOSTLANE_CLIENT: 'desktop', HOSTLANE_TOKEN: '' }
        })
        child.once('error', reject)
        child.once('close', (code) =>
          code === 0 ? resolve() : reject(new Error('导出失败，请检查守护进程状态。'))
        )
      })
      await file.sync()
    } finally {
      await file.close()
    }
    await rename(temp, filePath)
    return { ok: true, result: filePath }
  } catch (err) {
    return {
      ok: false,
      error: { kind: 'IO', message: err instanceof Error ? err.message : '导出失败' }
    }
  } finally {
    await rm(temp, { force: true }).catch(() => {})
  }
}
