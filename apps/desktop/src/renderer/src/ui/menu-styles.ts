/** Shared look for dropdown menus, context menus and select popovers. */
export const menuContentClass =
  'z-50 min-w-44 max-h-(--radix-popper-available-height) overflow-y-auto rounded-lg border border-border bg-bg-raised p-1 text-base text-fg shadow-popover outline-none ' +
  'data-[state=open]:animate-pop-in data-[state=closed]:animate-pop-out'

export const menuItemClass =
  'relative flex h-control w-full items-center gap-2 rounded-sm px-2 text-left outline-none select-none ' +
  'data-highlighted:bg-accent data-highlighted:text-accent-fg ' +
  'data-disabled:pointer-events-none data-disabled:opacity-45 ' +
  "[&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 [&_svg]:text-fg-muted data-highlighted:[&_svg]:text-current"

export const menuDangerItemClass =
  'text-danger-text data-highlighted:bg-danger data-highlighted:text-white [&_svg]:text-current'

export const menuLabelClass = 'px-2 pt-1.5 pb-1 text-xs font-medium text-fg-subtle'

export const menuSeparatorClass = '-mx-1 my-1 h-px bg-border'

export const menuShortcutClass =
  'ml-auto pl-4 text-sm text-fg-subtle tracking-wide group-data-highlighted:text-current'
