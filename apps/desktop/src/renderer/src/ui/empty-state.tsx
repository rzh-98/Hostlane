import type { ReactNode } from 'react'
import { cn } from '@/lib/cn'

export interface EmptyStateProps {
  icon?: ReactNode
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  className?: string
}

export function EmptyState({ icon, title, description, actions, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex h-full flex-col items-center justify-center gap-1 p-8 text-center',
        className
      )}
    >
      {icon && (
        <div className="mb-2 text-fg-subtle [&_svg:not([class*='size-'])]:size-8 [&_svg]:stroke-[1.5]">
          {icon}
        </div>
      )}
      <p className="text-md font-medium text-fg">{title}</p>
      {description && <p className="max-w-80 text-base text-fg-muted">{description}</p>}
      {actions && <div className="mt-3 flex items-center gap-2">{actions}</div>}
    </div>
  )
}
