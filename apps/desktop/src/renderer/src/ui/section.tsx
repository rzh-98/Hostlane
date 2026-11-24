import type { ComponentProps, ReactNode } from 'react'
import { cn } from '@/lib/cn'

export interface SectionProps extends Omit<ComponentProps<'section'>, 'title'> {
  title?: ReactNode
  description?: ReactNode
  /** Actions aligned with the title. */
  actions?: ReactNode
  /** Separate direct children with hairlines (good for <FormRow> stacks). */
  divided?: boolean
}

/** Titled group of content. Hierarchy comes from type and hairlines, not boxes. */
export function Section({
  title,
  description,
  actions,
  divided,
  className,
  children,
  ...props
}: SectionProps) {
  return (
    <section className={cn('flex flex-col', className)} {...props}>
      {(title || actions) && (
        <header className="flex items-end justify-between gap-4 pb-2">
          <div className="flex flex-col gap-0.5">
            {title && <h2 className="text-md font-semibold text-fg">{title}</h2>}
            {description && <p className="text-sm text-fg-muted">{description}</p>}
          </div>
          {actions && <div className="flex items-center gap-2">{actions}</div>}
        </header>
      )}
      <div
        className={cn('flex flex-col border-t border-border', divided && 'divide-y divide-border')}
      >
        {children}
      </div>
    </section>
  )
}
