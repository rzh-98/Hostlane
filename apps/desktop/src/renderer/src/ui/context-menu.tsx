import type { ComponentProps, ReactNode } from 'react'
import { ContextMenu as ContextMenuPrimitive } from 'radix-ui'
import { cn } from '@/lib/cn'
import {
  menuContentClass,
  menuDangerItemClass,
  menuItemClass,
  menuSeparatorClass,
  menuShortcutClass
} from './menu-styles'

/** Right-click menu with the same look and keyboard model as <Menu>. Also opens with Shift+F10 / the context-menu key. */
export const ContextMenu = ContextMenuPrimitive.Root
export const ContextMenuTrigger = ContextMenuPrimitive.Trigger

export function ContextMenuContent({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        collisionPadding={8}
        className={cn(
          menuContentClass,
          'origin-(--radix-context-menu-content-transform-origin)',
          className
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  )
}

export interface ContextMenuItemProps extends ComponentProps<typeof ContextMenuPrimitive.Item> {
  icon?: ReactNode
  shortcut?: string
  danger?: boolean
}

export function ContextMenuItem({
  icon,
  shortcut,
  danger,
  className,
  children,
  ...props
}: ContextMenuItemProps) {
  return (
    <ContextMenuPrimitive.Item
      className={cn('group', menuItemClass, danger && menuDangerItemClass, className)}
      {...props}
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
      {shortcut && <span className={menuShortcutClass}>{shortcut}</span>}
    </ContextMenuPrimitive.Item>
  )
}

export function ContextMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return <ContextMenuPrimitive.Separator className={cn(menuSeparatorClass, className)} {...props} />
}
