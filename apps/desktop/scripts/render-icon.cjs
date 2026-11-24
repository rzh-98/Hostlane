// Renders build/icon.svg to the PNG icons. Run with `npm run icon` (uses Electron's renderer).
const { app, BrowserWindow } = require('electron')
const { readFileSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')

const root = join(__dirname, '..')
const outputs = [
  { file: join(root, 'build', 'icon.png'), size: 1024 },
  { file: join(root, 'resources', 'icon.png'), size: 512 },
  { file: join(root, '..', '..', 'docs', 'icon.png'), size: 256 }
]

app.disableHardwareAcceleration()
app.on('window-all-closed', () => {})
async function render() {
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
