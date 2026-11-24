import type { ComponentProps } from 'react'
import { cn } from '@/lib/cn'

/** Vertical stack: <Toolbar/> on top, content filling the rest. */
export function Page({ className, ...props }: ComponentProps<'div'>) {
  return <div className={cn('flex h-full min-h-0 min-w-0 flex-1 flex-col', className)} {...props} />
}

const widths = {
  narrow: 'max-w-2xl',
  default: 'max-w-4xl',
  full: 'max-w-none'
} as const

export interface PageContentProps extends ComponentProps<'div'> {
  /** Max reading width of the content column. */
  width?: keyof typeof widths
  /** Set false for edge-to-edge layouts (lists, split views). */
  padded?: boolean
}

/** Scrollable main content area. */
export function PageContent({
  width = 'default',
  padded = true,
  className,
  children,
  ...props
}: PageContentProps) {
  if (!padded) {
    return (
      <div className={cn('flex min-h-0 flex-1 overflow-hidden', className)} {...props}>
        {children}
      </div>
    )
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" {...props}>
      <div className={cn('mx-auto flex w-full flex-col gap-8 p-page', widths[width], className)}>
        {children}
      </div>
    </div>
  )
}
