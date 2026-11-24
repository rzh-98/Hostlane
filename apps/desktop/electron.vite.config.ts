import { resolve } from 'node:path'
import { readFileSync } from 'node:fs'
import { defineConfig } from 'electron-vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const pkg = JSON.parse(readFileSync(resolve(__dirname, 'package.json'), 'utf-8'))
const appName: string = pkg.productName ?? pkg.name

export default defineConfig({
  main: {},
  preload: {},
  renderer: {
    resolve: {
      alias: {
        '@': resolve(__dirname, 'src/renderer/src'),
        '@shared': resolve(__dirname, 'src/shared')
      }
    },
    define: {
      __APP_NAME__: JSON.stringify(appName),
      __APP_VERSION__: JSON.stringify(pkg.version)
    },
    plugins: [react(), tailwindcss()]
  }
})
