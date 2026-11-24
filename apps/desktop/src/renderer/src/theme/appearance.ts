import { useSyncExternalStore } from 'react'
import { platform } from '@/lib/platform'
import { createStore } from '@/lib/store'

export type ThemePreference = 'system' | 'light' | 'dark'
export type Density = 'compact' | 'regular' | 'comfortable'

export interface Appearance {
  theme: ThemePreference
  density: Density
  /** Written to `--user-hue`, overriding `--brand-hue` from tokens.css; `null` keeps the brand default. */
  accentHue: number | null
}

export const defaultAppearance: Appearance = {
  theme: 'system',
  density: 'regular',
  accentHue: null
}

/** Persisted under the `appearance` localStorage key. */
const store = createStore<Appearance>('appearance', defaultAppearance)
const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')

export function resolveTheme(theme: ThemePreference): 'light' | 'dark' {
  if (theme === 'system') return darkQuery.matches ? 'dark' : 'light'
  return theme
}

/** Writes the current appearance onto <html> so CSS tokens can react to it. */
function apply(): void {
  const state = store.get()
  const root = document.documentElement
  root.dataset.platform = platform
  root.dataset.theme = resolveTheme(state.theme)
  root.dataset.density = state.density
  if (state.accentHue === null) root.style.removeProperty('--user-hue')
  else root.style.setProperty('--user-hue', String(state.accentHue))

  const toolbarHeight = parseFloat(getComputedStyle(root).getPropertyValue('--spacing-toolbar'))
  void window.desktop?.setTitleBarHeight(toolbarHeight)
}

/** Call once before rendering to avoid a flash of the wrong theme. */
export function initAppearance(): void {
  let themeSource = store.get().theme
  apply()
  void window.desktop?.setThemeSource(themeSource)

  store.subscribe(() => {
    const { theme } = store.get()
    if (theme !== themeSource) {
      themeSource = theme
      void window.desktop?.setThemeSource(theme)
    }
    apply()
  })
  darkQuery.addEventListener('change', () => {
    if (store.get().theme === 'system') apply()
  })
}

export const setAppearance = (patch: Partial<Appearance>): void => store.set(patch)
export const resetAppearance = (): void => store.reset()

function subscribeSystemTheme(listener: () => void): () => void {
  darkQuery.addEventListener('change', listener)
  return () => darkQuery.removeEventListener('change', listener)
}

/** Current appearance preferences plus the resolved light/dark theme. */
export function useAppearance(): Appearance & { resolvedTheme: 'light' | 'dark' } {
  const state = store.use()
  const systemDark = useSyncExternalStore(subscribeSystemTheme, () => darkQuery.matches)
  const resolvedTheme = state.theme === 'system' ? (systemDark ? 'dark' : 'light') : state.theme
  return { ...state, resolvedTheme }
}
