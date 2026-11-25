// Launches the packaged app with a throwaway data dir, waits for the renderer to mount and for
// the bundled daemon to come up, then saves a screenshot.
// Usage (in apps/desktop, after `npm run pack` or `dist:*`): node scripts/smoke.mjs <output.png>
import { execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const output = process.argv[2] ?? 'screenshot.png'
const port = 9333
const pkg = JSON.parse(readFileSync('package.json', 'utf8'))
const releaseDir = join('release', pkg.version)

function findApp() {
  if (process.platform === 'win32') {
    const dir = join(releaseDir, 'win-unpacked')
    return {
      executable: join(dir, `${pkg.productName}.exe`),
      hostlane: join(dir, 'resources', 'bin', 'hostlane.exe')
    }
  }
  if (process.platform === 'darwin') {
    const macDir = readdirSync(releaseDir).find((dir) => dir.startsWith('mac'))
    const bundle = join(releaseDir, macDir, `${pkg.productName}.app`, 'Contents')
    return {
      executable: join(bundle, 'MacOS', pkg.productName),
      hostlane: join(bundle, 'Resources', 'bin', 'hostlane')
    }
  }
  const dir = join(releaseDir, 'linux-unpacked')
  return { executable: join(dir, pkg.name), hostlane: join(dir, 'resources', 'bin', 'hostlane') }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function findPage() {
  for (let attempt = 0; attempt < 60; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()
      const page = targets.find((target) => target.type === 'page')
      if (page) return page
    } catch {}
    await sleep(500)
  }
  throw new Error('No renderer page found within 30s')
}

function connect(url) {
  const socket = new WebSocket(url)
  let nextId = 0
  const pending = new Map()
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data)
    const handler = pending.get(message.id)
    if (!handler) return
    pending.delete(message.id)
    message.error
      ? handler.reject(new Error(message.error.message))
      : handler.resolve(message.result)
  })
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++nextId
      pending.set(id, { resolve, reject })
      socket.send(JSON.stringify({ id, method, params }))
    })
  return new Promise((resolve, reject) => {
    socket.addEventListener('open', () => resolve({ send, close: () => socket.close() }))
    socket.addEventListener('error', () => reject(new Error('CDP connection failed')))
  })
}

function daemonStatus(hostlane, env) {
  try {
    return JSON.parse(execFileSync(hostlane, ['--json', 'status'], { env, encoding: 'utf8' }))
  } catch {
    return null
  }
}

const { executable, hostlane } = findApp()
for (const file of [executable, hostlane]) {
  if (!existsSync(file)) throw new Error(`Not found: ${file}`)
}
console.log(`Launching ${executable}`)

const work = mkdtempSync(join(tmpdir(), 'hostlane-smoke-'))
writeFileSync(join(work, 'ssh_config'), 'Host demo\n  HostName 192.0.2.10\n  User deploy\n')
const env = {
  ...process.env,
  HOSTLANE_SOCKET:
    process.platform === 'win32'
      ? `\\\\.\\pipe\\hostlane-smoke-${process.pid}`
      : join(work, 'hl.sock'),
  HOSTLANE_DATA_DIR: join(work, 'data'),
  HOSTLANE_SSH_CONFIG: join(work, 'ssh_config'),
  HOSTLANE_NOTIFY: '0'
}
const args = [`--remote-debugging-port=${port}`, `--user-data-dir=${join(work, 'electron')}`]
if (process.platform === 'linux') args.push('--no-sandbox')
const app = spawn(executable, args, { stdio: 'inherit', env })

try {
  const page = await findPage()
  const cdp = await connect(page.webSocketDebuggerUrl)
  let state
  for (let attempt = 0; attempt < 40; attempt++) {
    const { result } = await cdp.send('Runtime.evaluate', {
      expression: `JSON.stringify({
        title: document.title,
        mounted: (document.getElementById('root')?.childElementCount ?? 0) > 0,
        demo: document.body?.innerText.includes('demo') ?? false
      })`,
      returnByValue: true
    })
    state = typeof result.value === 'string' ? JSON.parse(result.value) : null
    if (state?.mounted && state.demo) break
    await sleep(500)
  }
  console.log(`Renderer state: ${JSON.stringify(state)}`)
  if (!state?.mounted) throw new Error('Renderer did not mount')
  if (!state.demo) throw new Error('Hosts page did not show the host from ssh_config')

  const denied = await cdp.send('Runtime.evaluate', {
    expression: `window.desktop.hostlane.call('secrets.set', {}).then(reply => reply.error?.kind)`,
    awaitPromise: true,
    returnByValue: true
  })
  if (denied.result.value !== 'METHOD_NOT_FOUND')
    throw new Error('Renderer credential IPC is not blocked')

  const status = daemonStatus(hostlane, env)
  console.log(`Daemon status: ${JSON.stringify(status)}`)
  if (!status?.pid) throw new Error('The app did not start the bundled daemon')

  await sleep(1000)
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' })
  writeFileSync(output, Buffer.from(data, 'base64'))
  console.log(`Saved ${output}`)
  cdp.close()
} finally {
  app.kill()
  try {
    execFileSync(hostlane, ['daemon', 'stop'], { env, stdio: 'ignore' })
  } catch {}
}
process.exit(0)
