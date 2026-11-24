import type { ComponentProps, ReactNode } from 'react'
import { Select as SelectPrimitive } from 'radix-ui'
import { Check, ChevronsUpDown } from 'lucide-react'
import { cn } from '@/lib/cn'
import { useFieldControl } from './field'
import { menuContentClass, menuItemClass } from './menu-styles'

export interface SelectOption {
  value: string
  label: ReactNode
  disabled?: boolean
}

export interface SelectProps extends Omit<ComponentProps<typeof SelectPrimitive.Root>, 'children'> {
  options: SelectOption[]
  placeholder?: string
  id?: string
  className?: string
  'aria-label'?: string
}

/** Native-feeling select: ↑↓ / typing to choose, Enter to confirm, Esc to cancel. */
export function Select({ options, placeholder, id, className, ...props }: SelectProps) {
  const fieldProps = useFieldControl({ id })
  return (
    <SelectPrimitive.Root {...props}>
      <SelectPrimitive.Trigger
        aria-label={props['aria-label']}
        {...fieldProps}
        className={cn(
          'inline-flex h-control min-w-32 items-center justify-between gap-2 rounded-md border border-border-strong bg-control pr-1.5 pl-2.5 text-base text-fg outline-none',
          'hover:bg-control-hover focus-visible:focus-ring data-placeholder:text-fg-subtle',
          'disabled:pointer-events-none disabled:opacity-45',
          className
        )}
      >
        <span className="truncate">
          <SelectPrimitive.Value placeholder={placeholder} />
        </span>
        <SelectPrimitive.Icon>
          <ChevronsUpDown className="size-3.5 text-fg-subtle" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          collisionPadding={8}
          className={cn(
            menuContentClass,
            'min-w-(--radix-select-trigger-width) origin-(--radix-select-content-transform-origin)'
          )}
        >
          <SelectPrimitive.Viewport>
            {options.map((option) => (
              <SelectPrimitive.Item
                key={option.value}
                value={option.value}
                disabled={option.disabled}
                className={cn(menuItemClass, 'pl-7')}
              >
                <span className="absolute left-2 flex size-4 items-center justify-center">
                  <SelectPrimitive.ItemIndicator>
                    <Check className="size-3.5" strokeWidth={2.5} />
                  </SelectPrimitive.ItemIndicator>
                </span>
                <SelectPrimitive.ItemText>{option.label}</SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}
