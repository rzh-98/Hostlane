import { defineConfig } from 'vite'
import { resolve } from 'node:path'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import pkg from './package.json' with { type: 'json' }

const target = process.env.TAURI_ENV_PLATFORM ?? process.platform
export default defineConfig({
  root: 'src/renderer',
  resolve: { alias: { '@': resolve('src/renderer/src'), '@shared': resolve('src/shared') } },
  define: {
    __APP_NAME__: JSON.stringify(pkg.productName),
    __APP_VERSION__: JSON.stringify(pkg.version),
    'import.meta.env.VITE_DESKTOP_PLATFORM': JSON.stringify(
      ({ windows: 'win32', macos: 'darwin' } as Record<string, string>)[target] ?? target
    )
  },
  server: { port: 1420, strictPort: true },
  build: { outDir: resolve('dist/tauri'), emptyOutDir: true },
  plugins: [react(), tailwindcss()]
})
