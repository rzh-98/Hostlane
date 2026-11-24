import type { ComponentProps, ReactNode } from 'react'
import { cn } from '@/lib/cn'

/** Navigation column. Place it in <AppShell sidebar={…}> or anywhere else. */
export function Sidebar({ className, ...props }: ComponentProps<'aside'>) {
  return (
    <aside
      className={cn('flex h-full flex-col border-r border-border bg-bg-subtle', className)}
      {...props}
    />
  )
}

/** Top strip aligned with the toolbar; doubles as the window drag area on macOS. */
export function SidebarHeader({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-drag-region
      className={cn(
        'flex h-toolbar shrink-0 items-center gap-2 px-3 mac:pl-[calc(var(--spacing-traffic-light)+8px)]',
        className
      )}
      {...props}
    />
  )
}

export function SidebarContent({ className, ...props }: ComponentProps<'nav'>) {
  return (
    <nav
      className={cn('flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-2 py-1', className)}
      {...props}
    />
  )
}

export function SidebarGroup({
  label,
  className,
  children,
  ...props
}: ComponentProps<'div'> & { label?: ReactNode }) {
  return (
    <div className={cn('flex flex-col gap-px', className)} {...props}>
      {label && <div className="px-2 pt-1 pb-1 text-xs font-medium text-fg-subtle">{label}</div>}
      {children}
    </div>
  )
}

export interface SidebarItemProps extends ComponentProps<'button'> {
  icon?: ReactNode
  active?: boolean
  /** Count or badge on the trailing edge. */
  trailing?: ReactNode
}

export function SidebarItem({
  icon,
  active,
  trailing,
  className,
  children,
  ...props
}: SidebarItemProps) {
  return (
    <button
      type="button"
      aria-current={active ? 'page' : undefined}
      className={cn(
        'group flex h-control w-full shrink-0 items-center gap-2 rounded-md px-2 text-left text-base text-fg outline-none transition-colors duration-100',
        'hover:bg-hover active:bg-active focus-visible:focus-ring-inset',
        'aria-[current=page]:bg-accent-subtle aria-[current=page]:font-medium aria-[current=page]:text-accent-text',
        'disabled:pointer-events-none disabled:opacity-45',
        "[&_svg:not([class*='size-'])]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-muted aria-[current=page]:[&_svg]:text-accent-text",
        className
      )}
      {...props}
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
      {trailing && <span className="text-sm text-fg-subtle">{trailing}</span>}
    </button>
  )
}

export function SidebarFooter({ className, ...props }: ComponentProps<'div'>) {
  return <div className={cn('flex shrink-0 flex-col gap-px p-2', className)} {...props} />
}
