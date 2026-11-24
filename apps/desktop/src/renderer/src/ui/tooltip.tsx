import type { ComponentProps, ReactNode } from 'react'
import { Tooltip as TooltipPrimitive } from 'radix-ui'
import { cn } from '@/lib/cn'
import { Kbd } from './kbd'

export function TooltipProvider(props: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={500} skipDelayDuration={200} {...props} />
}

export interface TooltipProps {
  content: ReactNode
  /** Keyboard shortcut shown after the label, e.g. "⌘B". */
  shortcut?: string
  side?: 'top' | 'right' | 'bottom' | 'left'
  children: ReactNode
}

export function Tooltip({ content, shortcut, side = 'bottom', children }: TooltipProps) {
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content
          side={side}
          sideOffset={6}
          collisionPadding={8}
          className={cn(
            'z-50 flex items-center gap-2 rounded-md border border-border bg-bg-raised px-2 py-1 text-sm text-fg shadow-popover',
            'data-[state=delayed-open]:animate-fade-in data-[state=closed]:animate-fade-out'
          )}
        >
          {content}
          {shortcut && <Kbd>{shortcut}</Kbd>}
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  )
}
