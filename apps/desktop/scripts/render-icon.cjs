// Renders build/icon.svg to the PNG icons. Run with `npm run icon` (uses Electron's renderer).
const { app, BrowserWindow, nativeImage } = require('electron')
const { mkdirSync, readFileSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')

const root = join(__dirname, '..')
const outputs = [
  { file: join(root, 'build', 'icon.png'), size: 1024 },
  { file: join(root, 'resources', 'icon.png'), size: 512 },
  { file: join(root, '..', '..', 'docs', 'icon.png'), size: 256 }
]

app.disableHardwareAcceleration()
app.on('window-all-closed', () => {})
function renderTray() {
  const image = nativeImage
    .createFromBuffer(readFileSync(outputs[2].file))
    .resize({ width: 32, height: 32 })
  const bitmap = image.toBitmap()
  if (bitmap.length !== 32 * 32 * 4) throw new Error('Unexpected tray bitmap size')
  for (let offset = 0; offset < bitmap.length; offset += 4)
    bitmap.subarray(offset, offset + 4).reverse()
  const directory = join(root, '..', '..', 'crates', 'hostlane-core', 'assets')
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, 'tray-32.argb'), bitmap)
}
async function render() {
  if (process.argv.includes('--tray-only')) return renderTray()
  const svg = readFileSync(join(root, 'build', 'icon.svg'), 'utf8')
  for (const { file, size } of outputs) {
    const win = new BrowserWindow({
      width: size,
      height: size,
      show: false,
      transparent: true,
      frame: false,
      useContentSize: true,
      webPreferences: { offscreen: true }
    })
    const sized = svg.replace('width="1024" height="1024"', `width="${size}" height="${size}"`)
    const html = `<html><body style="margin:0;background:transparent">${sized}</body></html>`
    await win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(html))
    await new Promise((resolve) => setTimeout(resolve, 300))
    const image = await win.webContents.capturePage({ x: 0, y: 0, width: size, height: size })
    writeFileSync(file, image.resize({ width: size, height: size }).toPNG())
    win.destroy()
    console.log(`${file} (${size}px)`)
  }
  renderTray()
}

app
  .whenReady()
  .then(render)
  .then(
    () => app.quit(),
    (error) => {
      console.error(error)
      app.exit(1)
    }
  )
