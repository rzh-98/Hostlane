import type { ComponentProps, ReactNode } from 'react'
import { DropdownMenu } from 'radix-ui'
import { Check, ChevronRight, Dot } from 'lucide-react'
import { cn } from '@/lib/cn'
import {
  menuContentClass,
  menuDangerItemClass,
  menuItemClass,
  menuLabelClass,
  menuSeparatorClass,
  menuShortcutClass
} from './menu-styles'

/*
 * Dropdown menu. Keyboard: Enter/Space/↓ opens, ↑↓ moves, → opens submenus,
 * ← / Esc closes, typing jumps to matching items. Focus returns to the trigger.
 */

export const Menu = DropdownMenu.Root
export const MenuTrigger = DropdownMenu.Trigger
export const MenuGroup = DropdownMenu.Group
export const MenuSub = DropdownMenu.Sub
export const MenuRadioGroup = DropdownMenu.RadioGroup

export function MenuContent({
  className,
  sideOffset = 4,
  align = 'start',
  ...props
}: ComponentProps<typeof DropdownMenu.Content>) {
  return (
    <DropdownMenu.Portal>
      <DropdownMenu.Content
        sideOffset={sideOffset}
        align={align}
        collisionPadding={8}
        className={cn(
          menuContentClass,
          'origin-(--radix-dropdown-menu-content-transform-origin)',
          className
        )}
        {...props}
      />
    </DropdownMenu.Portal>
  )
}

export interface MenuItemProps extends ComponentProps<typeof DropdownMenu.Item> {
  icon?: ReactNode
  shortcut?: string
  /** Red styling for destructive actions. */
  danger?: boolean
}

export function MenuItem({ icon, shortcut, danger, className, children, ...props }: MenuItemProps) {
  return (
    <DropdownMenu.Item
      className={cn('group', menuItemClass, danger && menuDangerItemClass, className)}
      {...props}
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
      {shortcut && <span className={menuShortcutClass}>{shortcut}</span>}
    </DropdownMenu.Item>
  )
}

export function MenuCheckboxItem({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenu.CheckboxItem>) {
  return (
    <DropdownMenu.CheckboxItem className={cn(menuItemClass, 'pl-7', className)} {...props}>
      <span className="absolute left-2 flex size-4 items-center justify-center">
        <DropdownMenu.ItemIndicator>
          <Check className="size-3.5" strokeWidth={2.5} />
        </DropdownMenu.ItemIndicator>
      </span>
      {children}
    </DropdownMenu.CheckboxItem>
  )
}

export function MenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenu.RadioItem>) {
  return (
    <DropdownMenu.RadioItem className={cn(menuItemClass, 'pl-7', className)} {...props}>
      <span className="absolute left-2 flex size-4 items-center justify-center">
        <DropdownMenu.ItemIndicator>
          <Dot className="size-6" strokeWidth={4} />
        </DropdownMenu.ItemIndicator>
      </span>
      {children}
    </DropdownMenu.RadioItem>
  )
}

export function MenuLabel({ className, ...props }: ComponentProps<typeof DropdownMenu.Label>) {
  return <DropdownMenu.Label className={cn(menuLabelClass, className)} {...props} />
}

export function MenuSeparator({
  className,
  ...props
}: ComponentProps<typeof DropdownMenu.Separator>) {
  return <DropdownMenu.Separator className={cn(menuSeparatorClass, className)} {...props} />
}

export function MenuSubTrigger({
  icon,
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenu.SubTrigger> & { icon?: ReactNode }) {
  return (
    <DropdownMenu.SubTrigger
      className={cn(menuItemClass, 'data-[state=open]:not-data-highlighted:bg-hover', className)}
      {...props}
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
      <ChevronRight className="ml-auto size-3.5!" />
    </DropdownMenu.SubTrigger>
  )
}

export function MenuSubContent({
  className,
  ...props
}: ComponentProps<typeof DropdownMenu.SubContent>) {
  return (
    <DropdownMenu.Portal>
      <DropdownMenu.SubContent
        sideOffset={2}
        collisionPadding={8}
        className={cn(
          menuContentClass,
          'origin-(--radix-dropdown-menu-content-transform-origin)',
          className
        )}
        {...props}
      />
    </DropdownMenu.Portal>
  )
}
