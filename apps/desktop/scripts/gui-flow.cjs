const assert = require('node:assert/strict')
const childProcess = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { app, BrowserWindow, dialog } = require('electron')

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'hl-gui-'))
const binary = path.resolve(
  '../../target/debug',
  process.platform === 'win32' ? 'hostlane.exe' : 'hostlane'
)
Object.assign(process.env, {
  HOME: work,
  USERPROFILE: work,
  CODEX_HOME: path.join(work, 'codex'),
  CLAUDE_CONFIG_DIR: path.join(work, 'claude'),
  HOSTLANE_SOCKET:
    process.platform === 'win32' ? `\\\\.\\pipe\\hl-gui-${process.pid}` : path.join(work, 's'),
  HOSTLANE_DATA_DIR: path.join(work, 'data'),
  HOSTLANE_SSH_CONFIG: path.join(work, 'config'),
  HOSTLANE_SECRET_STORE: 'file',
  HOSTLANE_BIN: binary,
  HOSTLANE_NOTIFY: '0'
})
for (const name of ['HOSTLANE_TOKEN', 'HOSTLANE_CLIENT', 'ELECTRON_RENDERER_URL'])
  delete process.env[name]
fs.writeFileSync(process.env.HOSTLANE_SSH_CONFIG, 'Host fixture\n  HostName localhost\n')
app.setPath('userData', path.join(work, 'electron'))

const fixture = '  ui-only test value  '
let cancel = true
const exec = childProcess.execFile
childProcess.execFile = (program, args, options, callback) => {
  if (
    !['osascript', 'powershell.exe', 'zenity', 'kdialog'].includes(
      path.basename(program).toLowerCase()
    ) ||
    !options?.env?.HOSTLANE_DIALOG_LABEL
  )
    return exec(program, args, options, callback)
  assert.ok(options.env.HOSTLANE_DIALOG_LABEL.includes('fixture'))
  assert.ok(!args.join(' ').includes(fixture))
  if (process.platform === 'win32' && process.env.GITHUB_ACTIONS === 'true') {
    const original = Buffer.from(args.at(-1), 'base64').toString('utf16le')
    const marker = 'if ($w.ShowDialog() -eq $true)'
    assert.ok(original.includes(marker))
    const response = cancel
      ? '$w.DialogResult=$false'
      : '$box.Password=$env:HL_GUI_FIXTURE; $w.DialogResult=$true'
    const script = original.replace(marker, `$w.Add_ContentRendered({${response}}); ${marker}`)
    return exec(
      program,
      [...args.slice(0, -1), Buffer.from(script, 'utf16le').toString('base64')],
      { ...options, env: { ...options.env, HL_GUI_FIXTURE: fixture }, timeout: 15000 },
      callback
    )
  }
  queueMicrotask(() => callback(null, Buffer.from(cancel ? '\n' : `${fixture}\n`), Buffer.alloc(0)))
}
const destination = path.join(work, 'audit.csv')
dialog.showSaveDialog = async () => ({ canceled: cancel, filePath: destination })
dialog.showMessageBox = async () => ({ response: cancel ? 0 : 1 })
dialog.showOpenDialog = async () => ({
  canceled: false,
  filePaths: [path.join(work, 'fixture-key')]
})
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function test() {
  const win = BrowserWindow.getAllWindows()[0]
  const evaluate = (expression) => win.webContents.executeJavaScript(expression)
  const rpc = (method, params) =>
    evaluate(`window.desktop.hostlane.call(${JSON.stringify(method)},${JSON.stringify(params)})`)
  const click = async (label) => {
    for (let attempt = 0; attempt < 50; attempt++) {
      if (
        await evaluate(
          `(()=>{const btn=[...document.querySelectorAll('button')].find(node=>!node.disabled && node.textContent.trim()===${JSON.stringify(label)}); if(!btn)return false;btn.click();return true})()`
        )
      )
        return
      await sleep(100)
    }
    throw new Error(`Button not found: ${label}`)
  }
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await evaluate(`document.body?.innerText.includes('fixture')`)) break
    await sleep(150)
  }
  assert.ok((await rpc('hosts.list', {})).result.hosts.some((host) => host.id === 'fixture'))
  assert.equal((await rpc('secrets.set', {})).error.kind, 'METHOD_NOT_FOUND')
  assert.equal(
    await evaluate('window.desktop.hostlane.chooseKey()'),
    path.join(work, 'fixture-key')
  )
  const prompt = () => evaluate(`window.desktop.hostlane.promptSecret('fixture','password')`)
  assert.deepEqual(await prompt(), { ok: true, result: false })
  assert.equal((await rpc('secrets.list', {})).result.secrets.length, 0)
  assert.equal((await evaluate(`window.desktop.setup.configureMcp('codex')`)).result, null)
  assert.ok(!fs.existsSync(path.join(process.env.CODEX_HOME, 'config.toml')))
  assert.equal((await evaluate(`window.desktop.hostlane.exportAudit('csv',{})`)).result, null)
  assert.ok(!fs.existsSync(destination))
  cancel = false
  await click('保存密码…')
  await click('打开安全输入窗口')
  for (let attempt = 0; attempt < 80; attempt++) {
    if (await evaluate(`document.body.innerText.includes('凭据已保存到系统钥匙串')`)) break
    await sleep(100)
  }
  assert.ok(await evaluate(`document.body.innerText.includes('凭据已保存到系统钥匙串')`))
  assert.equal((await rpc('secrets.list', {})).result.secrets.length, 1)
  const saved = JSON.parse(
    fs.readFileSync(path.join(process.env.HOSTLANE_DATA_DIR, 'secrets.json'), 'utf8')
  )
  assert.ok(Object.values(saved).includes(fixture))
  const audit = await evaluate(
    `window.desktop.hostlane.exportAudit('csv',{host:'fixture',client:'desktop',since:0})`
  )
  assert.deepEqual(audit, { ok: true, result: destination })
  const csv = fs.readFileSync(destination, 'utf8')
  assert.ok(csv.includes('secrets.set') && !csv.includes(fixture))
  const mcp = await evaluate(`window.desktop.setup.configureMcp('codex')`)
  assert.ok(mcp.ok)
  assert.ok(fs.readFileSync(mcp.result, 'utf8').includes('[mcp_servers.hostlane]'))
  const claude = await evaluate(`window.desktop.setup.configureMcp('claude')`)
  assert.ok(claude.ok)
  assert.deepEqual(JSON.parse(fs.readFileSync(claude.result, 'utf8')).mcpServers.hostlane.args, [
    'mcp'
  ])
  assert.deepEqual((await rpc('files.rollback_plan', { task: 'empty-task' })).result, [])
  assert.ok(!(await evaluate(`document.body.innerText.includes(${JSON.stringify(fixture)})`)))
}

async function finish() {
  let status = 0
  try {
    await test()
    console.log(
      'GUI actions: credential save/cancel, key picker, MCP setup, filtered export and rollback preview passed'
    )
  } catch (err) {
    console.error(err)
    status = 1
  }
  try {
    childProcess.execFileSync(binary, ['daemon', 'stop'], { stdio: 'ignore' })
  } catch {}
  app.exit(status)
}
app.once('browser-window-created', (_event, win) =>
  win.webContents.once('did-finish-load', () => void finish())
)
require('../out/main/index.js')
