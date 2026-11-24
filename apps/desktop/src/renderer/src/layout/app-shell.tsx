import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode
} from 'react'
import { cn } from '@/lib/cn'
import { isModKey } from '@/lib/platform'
import { readJson, writeJson } from '@/lib/storage'

interface ShellContextValue {
  /** True when the shell renders a sidebar at all. */
  hasSidebar: boolean
  sidebarOpen: boolean
  setSidebarOpen: (open: boolean) => void
  toggleSidebar: () => void
}

const ShellContext = createContext<ShellContextValue>({
  hasSidebar: false,
  sidebarOpen: false,
  setSidebarOpen: () => {},
  toggleSidebar: () => {}
})

/** Shell state for toolbars and pages. Safe to call outside <AppShell>. */
export function useShell(): ShellContextValue {
  return useContext(ShellContext)
}

const STORAGE_KEY = 'shell'
const MIN_WIDTH = 180
const MAX_WIDTH = 360

interface ShellState {
  sidebarOpen: boolean
  /** User-resized width in px; null follows the `--spacing-sidebar` token. */
  sidebarWidth: number | null
}

export interface AppShellProps {
  /** Optional sidebar. Omit it and the shell is just a full-width content area. */
  sidebar?: ReactNode
  /** Allow dragging the sidebar edge to resize it. */
  resizableSidebar?: boolean
  children: ReactNode
}

/**
 * Window-level layout: [sidebar] | main. Toggle the sidebar with ⌘B / Ctrl+B.
 * Each piece (Sidebar, Toolbar, PageContent) also works on its own.
 */
export function AppShell({ sidebar, resizableSidebar = true, children }: AppShellProps) {
  const [state, setState] = useState<ShellState>(() =>
    readJson<ShellState>(STORAGE_KEY, { sidebarOpen: true, sidebarWidth: null })
  )

  useEffect(() => writeJson(STORAGE_KEY, state), [state])

  const setSidebarOpen = useCallback(
    (open: boolean) => setState((prev) => ({ ...prev, sidebarOpen: open })),
    []
  )
  const toggleSidebar = useCallback(
    () => setState((prev) => ({ ...prev, sidebarOpen: !prev.sidebarOpen })),
    []
  )
  const setWidth = useCallback(
    (width: number | null) =>
      setState((prev) => ({
        ...prev,
        sidebarWidth: width === null ? null : Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, width))
      })),
    []
  )

  const hasSidebar = Boolean(sidebar)

  useEffect(() => {
    if (!hasSidebar) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (isModKey(event) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'b') {
        event.preventDefault()
        toggleSidebar()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [hasSidebar, toggleSidebar])

  const context = useMemo(
    () => ({
      hasSidebar,
      sidebarOpen: hasSidebar && state.sidebarOpen,
      setSidebarOpen,
      toggleSidebar
    }),
    [hasSidebar, state.sidebarOpen, setSidebarOpen, toggleSidebar]
  )

  const width = state.sidebarWidth === null ? 'var(--spacing-sidebar)' : `${state.sidebarWidth}px`

  return (
    <ShellContext.Provider value={context}>
      <div className="flex h-full w-full overflow-hidden bg-bg text-fg">
        {sidebar && (
          <div
            className="relative shrink-0 overflow-hidden transition-[width] duration-200 ease-out"
            style={{ width: context.sidebarOpen ? width : 0 }}
            inert={!context.sidebarOpen}
          >
            <div className="h-full" style={{ width }}>
              {sidebar}
            </div>
            {resizableSidebar && context.sidebarOpen && (
              <ResizeHandle width={width} onResize={setWidth} />
            )}
          </div>
        )}
        <main className="flex min-w-0 flex-1 flex-col">{children}</main>
      </div>
    </ShellContext.Provider>
  )
}

function ResizeHandle({
  width,
  onResize
}: {
  width: string
  onResize: (width: number | null) => void
}) {
  const [dragging, setDragging] = useState(false)

  const currentWidth = (element: HTMLElement) =>
    element.parentElement?.getBoundingClientRect().width ?? MIN_WIDTH

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="调整侧栏宽度"
      aria-valuemin={MIN_WIDTH}
      aria-valuemax={MAX_WIDTH}
      aria-valuenow={parseInt(width) || undefined}
      tabIndex={0}
      title="拖动调整宽度，双击恢复默认"
      onPointerDown={(event) => {
        event.preventDefault()
        const handle = event.currentTarget
        handle.setPointerCapture(event.pointerId)
        const startX = event.clientX
        const startWidth = currentWidth(handle)
        setDragging(true)
        const onMove = (moveEvent: PointerEvent) =>
          onResize(startWidth + moveEvent.clientX - startX)
        const onUp = () => {
          setDragging(false)
          handle.removeEventListener('pointermove', onMove)
          handle.removeEventListener('pointerup', onUp)
        }
        handle.addEventListener('pointermove', onMove)
        handle.addEventListener('pointerup', onUp)
      }}
      onDoubleClick={() => onResize(null)}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 32 : 8
        if (event.key === 'ArrowLeft') onResize(currentWidth(event.currentTarget) - step)
        else if (event.key === 'ArrowRight') onResize(currentWidth(event.currentTarget) + step)
        else return
        event.preventDefault()
      }}
      className={cn(
        'absolute inset-y-0 -right-1 z-10 w-2 cursor-col-resize outline-none',
        'after:absolute after:inset-y-0 after:left-1/2 after:w-0.5 after:-translate-x-1/2 after:transition-colors',
        'hover:after:bg-accent focus-visible:after:bg-accent',
        dragging && 'after:bg-accent'
      )}
    />
  )
}
