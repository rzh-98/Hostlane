import type { ReactNode } from 'react'
import { ToggleGroup } from 'radix-ui'
import { cn } from '@/lib/cn'
import { useFieldControl } from './field'

export interface SegmentedOption<T extends string> {
  value: T
  label: ReactNode
  icon?: ReactNode
  disabled?: boolean
}

export interface SegmentedControlProps<T extends string> {
  value: T
  onValueChange: (value: T) => void
  options: SegmentedOption<T>[]
  'aria-label'?: string
  className?: string
}

/** A small set of mutually exclusive options. ←/→ move between segments. */
export function SegmentedControl<T extends string>({
  value,
  onValueChange,
  options,
  className,
  ...props
}: SegmentedControlProps<T>) {
  const fieldProps = useFieldControl({})
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      onValueChange={(next) => next && onValueChange(next as T)}
      aria-label={props['aria-label']}
      {...fieldProps}
      className={cn(
        'inline-flex h-control items-center gap-0.5 rounded-md border border-border-strong bg-bg-subtle p-0.5',
        className
      )}
    >
      {options.map((option) => (
        <ToggleGroup.Item
          key={option.value}
          value={option.value}
          disabled={option.disabled}
          className={cn(
            'inline-flex h-full items-center gap-1.5 rounded-[calc(var(--radius-md)-2px)] px-2.5 text-base text-fg-muted outline-none transition-colors duration-100',
            'hover:text-fg focus-visible:focus-ring',
            'data-[state=on]:bg-control data-[state=on]:text-fg data-[state=on]:shadow-[0_0_0_0.5px_var(--color-border-strong),0_1px_2px_oklch(0_0_0/0.08)]',
            'disabled:pointer-events-none disabled:opacity-45',
            "[&_svg:not([class*='size-'])]:size-3.5"
          )}
        >
          {option.icon}
          {option.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  )
}
