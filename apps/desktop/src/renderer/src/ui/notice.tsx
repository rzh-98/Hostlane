import type { ComponentProps, ReactNode } from 'react'
import { X } from 'lucide-react'
import { cn } from '@/lib/cn'
import { buttonClass } from './button'

const tones = {
  neutral: 'bg-bg-subtle text-fg [&>svg]:text-fg-muted',
  success: 'bg-success-subtle text-fg [&>svg]:text-success-text',
  warning: 'bg-warning-subtle text-fg [&>svg]:text-warning-text',
  danger: 'bg-danger-subtle text-fg [&>svg]:text-danger-text'
} as const

export interface NoticeProps extends Omit<ComponentProps<'div'>, 'title'> {
  tone?: keyof typeof tones
  icon?: ReactNode
  /** Buttons at the end, e.g. “撤销”. */
  actions?: ReactNode
  /** Shows a × button. */
  onDismiss?: () => void
}

/**
 * Full-width inline feedback strip placed under a toolbar: results of an action,
 * recoverable errors. Announced to screen readers; errors interrupt politely.
 */
export function Notice({
  tone = 'neutral',
  icon,
  actions,
  onDismiss,
  className,
  children,
  ...props
}: NoticeProps) {
  return (
    <div
      role={tone === 'danger' ? 'alert' : 'status'}
      className={cn(
        'flex min-h-row shrink-0 items-center gap-2 border-b border-border px-3 py-1 text-base',
        "[&>svg]:shrink-0 [&>svg:not([class*='size-'])]:size-4",
        tones[tone],
        className
      )}
      {...props}
    >
      {icon}
      <div className="selectable min-w-0 flex-1">{children}</div>
      {actions && <div className="flex shrink-0 items-center gap-1">{actions}</div>}
      {onDismiss && (
        <button
          type="button"
          aria-label="关闭提示"
          onClick={onDismiss}
          className={cn(buttonClass('ghost', 'icon-sm'), 'shrink-0')}
        >
          <X />
        </button>
      )}
    </div>
  )
}
