import { execFile } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import type { HostlaneReply } from '../shared/ipc'
import type { HostsList } from '../shared/protocol/HostsList'
import { callDaemon } from './daemon'

const macPrompt = `try
  activate
  set answer to display dialog (system attribute "HOSTLANE_DIALOG_LABEL") with title "Hostlane · SSH" default answer "" with hidden answer buttons {"取消", "保存"} default button "保存" cancel button "取消"
  return text returned of answer
on error number -128
  return ""
end try`

const windowsPrompt = `$ErrorActionPreference='Stop'; Add-Type -AssemblyName PresentationFramework; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$w=New-Object Windows.Window; $w.Title='Hostlane · SSH'; $w.SizeToContent='WidthAndHeight'; $w.WindowStartupLocation='CenterScreen'; $w.ResizeMode='NoResize'; $w.Topmost=$true
$panel=New-Object Windows.Controls.StackPanel; $panel.Margin=18; $panel.Width=360; $w.Content=$panel
$label=New-Object Windows.Controls.TextBlock; $label.Text=$env:HOSTLANE_DIALOG_LABEL; $label.TextWrapping='Wrap'; [void]$panel.Children.Add($label)
$box=New-Object Windows.Controls.PasswordBox; $box.Margin='0,10,0,10'; [void]$panel.Children.Add($box)
$ok=New-Object Windows.Controls.Button; $ok.Content='保存'; $ok.IsDefault=$true; $ok.Add_Click({$w.DialogResult=$true}); [void]$panel.Children.Add($ok)
$cancel=New-Object Windows.Controls.Button; $cancel.Content='取消'; $cancel.IsCancel=$true; [void]$panel.Children.Add($cancel); $w.Add_ContentRendered({[void]$box.Focus()})
if ($w.ShowDialog() -eq $true) {[Console]::WriteLine($box.Password)}; $box.Clear()`

function input(label: string, signal: AbortSignal): Promise<Buffer | null> {
  let program: string
  let args: string[]
  let cancelCode: number | undefined
  if (process.platform === 'darwin') {
    program = '/usr/bin/osascript'
    args = ['-e', macPrompt]
  } else if (process.platform === 'win32') {
    program = join(
      process.env.SystemRoot ?? 'C:\\Windows',
      'System32/WindowsPowerShell/v1.0/powershell.exe'
    )
    args = [
      '-NoProfile',
      '-NonInteractive',
      '-STA',
      '-EncodedCommand',
      Buffer.from(windowsPrompt, 'utf16le').toString('base64')
    ]
  } else if (existsSync('/usr/bin/zenity')) {
    program = '/usr/bin/zenity'
    args = ['--password', `--title=Hostlane · ${label.replaceAll('\n', ' · ')}`]
    cancelCode = 1
  } else if (existsSync('/usr/bin/kdialog')) {
    program = '/usr/bin/kdialog'
    args = [
      '--title',
      'Hostlane · SSH',
      '--password',
      label.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    ]
    cancelCode = 1
  } else {
    throw new Error('请先在系统软件中心安装 Zenity 或 KDialog，再点击保存。')
  }
  return new Promise((resolve, reject) => {
    execFile(
      program,
      args,
      {
        windowsHide: true,
        encoding: 'buffer',
        signal,
        timeout: 180000,
        maxBuffer: 32768,
        env: { ...process.env, HOSTLANE_DIALOG_LABEL: label }
      },
      (error, stdout, stderr) => {
        stderr.fill(0)
        if (error) {
          stdout.fill(0)
          if (signal.aborted || (cancelCode !== undefined && error.code === cancelCode))
            resolve(null)
          else reject(new Error('无法打开系统密码窗口，或等待输入已超时。'))
        } else resolve(stdout)
      }
    )
  })
}

let active = false
export async function promptSecret(
  host: unknown,
  kind: unknown,
  key: unknown,
  signal: AbortSignal
): Promise<HostlaneReply<boolean>> {
  if (
    typeof host !== 'string' ||
    /[\x00-\x1f\x7f]/.test(host) ||
    (kind !== 'password' && kind !== 'passphrase') ||
    (key !== undefined &&
      (kind !== 'passphrase' || typeof key !== 'string' || /[\x00-\x1f\x7f]/.test(key)))
  )
    return { ok: false, error: { kind: 'INVALID_PARAMS', message: '无效的主机或密钥' } }
  if (active) return { ok: false, error: { kind: 'IO', message: '已有一个密码窗口打开。' } }
  active = true
  let bytes: Buffer | null = null
  try {
    const hosts = await callDaemon<HostsList>('hosts.list')
    if (!hosts.ok) return hosts
    const entry = hosts.result.hosts.find((item) => item.id === host)
    if (!entry)
      return { ok: false, error: { kind: 'HOST_NOT_FOUND', message: '主机已不存在，请刷新。' } }
    bytes = await input(
      `${kind === 'password' ? '登录密码' : '密钥口令'} · ${host} (${entry.hostname.replace(/[\x00-\x1f\x7f]/g, ' ')})${key ? `\n密钥：${key}` : ''}`,
      signal
    )
    const secret = bytes?.toString('utf8').replace(/\r?\n$/, '')
    if (!secret || signal.aborted) return { ok: true, result: false }
    const saved = await callDaemon('secrets.set', { host, kind, key_path: key, secret })
    return saved.ok
      ? { ok: true, result: true }
      : {
          ok: false,
          error: {
            kind: saved.error.kind,
            message:
              saved.error.kind === 'INVALID_PARAMS'
                ? '保存失败，请检查密钥文件、口令和长度。'
                : '未能保存，请检查连接和系统钥匙串。'
          }
        }
  } catch (err) {
    return {
      ok: false,
      error: { kind: 'IO', message: err instanceof Error ? err.message : '系统密码窗口不可用。' }
    }
  } finally {
    bytes?.fill(0)
    active = false
  }
}
