import type { ComponentProps } from 'react'
import { cn } from '@/lib/cn'

export function Kbd({ className, ...props }: ComponentProps<'kbd'>) {
  return (
    <kbd
      className={cn(
        'inline-flex h-4.5 min-w-4.5 items-center justify-center rounded-sm px-1 font-sans text-xs text-fg-subtle',
        className
      )}
      {...props}
    />
  )
}
