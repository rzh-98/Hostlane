import type { ComponentProps, ReactNode } from 'react'
import { PanelLeft } from 'lucide-react'
import { cn } from '@/lib/cn'
import { modKey } from '@/lib/platform'
import { IconButton } from '@/ui'
import { useShell } from './app-shell'

export interface ToolbarProps extends Omit<ComponentProps<'header'>, 'title'> {
  title?: ReactNode
  /** Secondary text after the title, e.g. a count. */
  subtitle?: ReactNode
  /** Content before the title (e.g. back button). */
  leading?: ReactNode
  /** Show the sidebar toggle when inside an <AppShell> with a sidebar. */
  sidebarToggle?: boolean
}

/**
 * Page toolbar: [sidebar toggle] [leading] title … children (actions).
 * On macOS it is the window's drag area and makes room for the traffic lights
 * whenever it is the leftmost element of the window.
 */
export function Toolbar({
  title,
  subtitle,
  leading,
  sidebarToggle = true,
  className,
  children,
  ...props
}: ToolbarProps) {
  const shell = useShell()
  const isLeftmost = !shell.sidebarOpen

  return (
    <header
      data-drag-region
      className={cn(
        'flex h-toolbar shrink-0 items-center gap-1 border-b border-border bg-bg px-2 transition-[padding] duration-200',
        isLeftmost && 'mac:pl-[calc(var(--spacing-traffic-light)+4px)]',
        className
      )}
      {...props}
    >
      {sidebarToggle && shell.hasSidebar && (
        <IconButton
          label={shell.sidebarOpen ? '隐藏侧栏' : '显示侧栏'}
          shortcut={`${modKey}B`}
          icon={<PanelLeft />}
          onClick={shell.toggleSidebar}
        />
      )}
      {leading}
      {title && (
        <div className="flex min-w-0 items-baseline gap-2 px-1.5">
          <h1 className="truncate text-md font-semibold text-fg">{title}</h1>
          {subtitle && <span className="truncate text-sm text-fg-subtle">{subtitle}</span>}
        </div>
      )}
      <div className="flex min-w-0 flex-1 items-center justify-end gap-1">{children}</div>
    </header>
  )
}

/** Thin vertical divider between toolbar groups. */
export function ToolbarSeparator() {
  return <div aria-hidden className="mx-1 h-4 w-px shrink-0 bg-border" />
}
