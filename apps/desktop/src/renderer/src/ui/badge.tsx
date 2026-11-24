import type { ComponentProps } from 'react'
import { cn } from '@/lib/cn'

const tones = {
  neutral: 'bg-hover text-fg-muted',
  accent: 'bg-accent-subtle text-accent-text',
  success: 'bg-success-subtle text-success-text',
  warning: 'bg-warning-subtle text-warning-text',
  danger: 'bg-danger-subtle text-danger-text'
} as const

export interface BadgeProps extends ComponentProps<'span'> {
  tone?: keyof typeof tones
}

export function Badge({ tone = 'neutral', className, ...props }: BadgeProps) {
  return (
    <span
      className={cn(
        'inline-flex h-5 shrink-0 items-center gap-1 rounded-sm px-1.5 text-xs font-medium whitespace-nowrap',
        tones[tone],
        className
      )}
      {...props}
    />
  )
}

/** Small colored dot for status. */
export function StatusDot({
  tone = 'neutral',
  className
}: {
  tone?: keyof typeof tones
  className?: string
}) {
  const color = {
    neutral: 'bg-fg-subtle',
    accent: 'bg-accent',
    success: 'bg-success',
    warning: 'bg-warning',
    danger: 'bg-danger'
  }[tone]
  return (
    <span
      aria-hidden
      className={cn('inline-block size-2 shrink-0 rounded-full', color, className)}
    />
  )
}
