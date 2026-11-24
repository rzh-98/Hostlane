import {
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent,
  type ReactNode
} from 'react'
import { cn } from '@/lib/cn'

/*
 * Single-select list (ARIA listbox) with roving focus.
 * Keyboard: ↑/↓ move selection, Home/End jump, Enter triggers onAction.
 * Mouse: click selects, double-click triggers onAction.
 */

interface ListContextValue {
  value: string | null
  select: (value: string) => void
  action?: (value: string) => void
}

const ListContext = createContext<ListContextValue | null>(null)

export interface ListProps extends Omit<ComponentProps<'div'>, 'onChange'> {
  value: string | null
  onValueChange: (value: string) => void
  /** Open / activate an item (Enter or double-click). */
  onAction?: (value: string) => void
  /** Required for screen readers when there is no visible heading. */
  'aria-label'?: string
}

function options(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>('[role="option"]:not([aria-disabled="true"])')
  )
}

export function List({
  value,
  onValueChange,
  onAction,
  className,
  onKeyDown,
  children,
  ref: forwardedRef,
  ...props
}: ListProps) {
  const select = (next: string) => onValueChange(next)
  const ref = useRef<HTMLDivElement>(null)
  const setRef = (node: HTMLDivElement | null) => {
    ref.current = node
    if (typeof forwardedRef === 'function') forwardedRef(node)
    else if (forwardedRef) forwardedRef.current = node
  }
  // If the selected item is not rendered (e.g. filtered out), the list itself stays tabbable.
  const [hasTabbableItem, setHasTabbableItem] = useState(false)
  useLayoutEffect(() => {
    const found = Boolean(ref.current?.querySelector('[role="option"][tabindex="0"]'))
    if (found !== hasTabbableItem) setHasTabbableItem(found)
  })

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event)
    if (event.defaultPrevented) return
    const items = options(event.currentTarget)
    if (items.length === 0) return
    const current = items.findIndex((item) => item === document.activeElement)
    let next = -1
    switch (event.key) {
      case 'ArrowDown':
        next = current < 0 ? 0 : Math.min(current + 1, items.length - 1)
        break
      case 'ArrowUp':
        next = current < 0 ? 0 : Math.max(current - 1, 0)
        break
      case 'Home':
        next = 0
        break
      case 'End':
        next = items.length - 1
        break
      case 'Enter':
        if (current >= 0 && onAction) {
          event.preventDefault()
          onAction(items[current].dataset.value!)
        }
        return
      default:
        return
    }
    event.preventDefault()
    const target = items[next]
    target.focus()
    target.scrollIntoView({ block: 'nearest' })
    select(target.dataset.value!)
  }

  return (
    <ListContext.Provider value={{ value, select, action: onAction }}>
      <div
        ref={setRef}
        role="listbox"
        aria-orientation="vertical"
        tabIndex={hasTabbableItem ? -1 : 0}
        onFocus={(event) => {
          if (event.target !== event.currentTarget) return
          const first = options(event.currentTarget)[0]
          if (first) {
            first.focus()
            select(first.dataset.value!)
          }
        }}
        onKeyDown={handleKeyDown}
        className={cn('flex flex-col gap-px outline-none', className)}
        {...props}
      >
        {children}
      </div>
    </ListContext.Provider>
  )
}

export interface ListItemProps extends Omit<ComponentProps<'div'>, 'title'> {
  value: string
  title: ReactNode
  description?: ReactNode
  /** Icon or avatar at the start. */
  leading?: ReactNode
  /** Meta text or badge at the end. */
  trailing?: ReactNode
  disabled?: boolean
}

export function ListItem({
  value,
  title,
  description,
  leading,
  trailing,
  disabled,
  className,
  onClick,
  onDoubleClick,
  onContextMenu,
  ...props
}: ListItemProps) {
  const list = useContext(ListContext)
  if (!list) throw new Error('<ListItem> must be used inside <List>')
  const selected = list.value === value

  return (
    <div
      role="option"
      aria-selected={selected}
      aria-disabled={disabled || undefined}
      data-value={value}
      tabIndex={selected ? 0 : -1}
      onClick={(event) => {
        onClick?.(event)
        if (!disabled) list.select(value)
      }}
      onDoubleClick={(event) => {
        onDoubleClick?.(event)
        if (!disabled) list.action?.(value)
      }}
      onContextMenu={(event) => {
        if (!disabled) list.select(value)
        onContextMenu?.(event)
      }}
      className={cn(
        'group/item relative flex min-h-row items-center gap-2.5 rounded-md px-2 outline-none select-none',
        description ? 'py-1.5' : 'py-1',
        'hover:bg-hover aria-selected:bg-accent-subtle aria-selected:hover:bg-accent-subtle',
        'focus-visible:focus-ring-inset aria-disabled:pointer-events-none aria-disabled:opacity-45',
        "[&_svg:not([class*='size-'])]:size-4",
        className
      )}
      {...props}
    >
      {leading && (
        <span className="flex shrink-0 items-center text-fg-muted group-aria-selected/item:text-accent-text">
          {leading}
        </span>
      )}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-base text-fg">{title}</span>
        {description && <span className="truncate text-sm text-fg-muted">{description}</span>}
      </span>
      {trailing && (
        <span className="flex shrink-0 items-center gap-2 text-sm text-fg-subtle">{trailing}</span>
      )}
    </div>
  )
}

/** Small heading that groups list items. */
export function ListHeader({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      role="presentation"
      className={cn('px-2 pt-3 pb-1 text-xs font-medium text-fg-subtle first:pt-1', className)}
      {...props}
    />
  )
}
