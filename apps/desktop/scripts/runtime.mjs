import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { resolve } from 'node:path'

const release = process.argv[2] === 'release'
const args = ['build', '--locked', '-p', 'hostlane-cli']
if (release) args.push('--release')
execFileSync('cargo', args, { cwd: '../..', stdio: 'inherit' })
mkdirSync('src-tauri/runtime', { recursive: true })
for (const file of process.platform === 'win32'
  ? ['hostlane.exe', 'hostlane-background.exe']
  : ['hostlane']) {
  copyFileSync(
    resolve('../../target', release ? 'release' : 'debug', file),
    resolve('src-tauri/runtime', file)
  )
}
