import type { ComponentProps, ReactNode } from 'react'
import { cn } from '@/lib/cn'
import { useFieldControl } from './field'

const frame =
  'rounded-md border border-border-strong bg-control text-base text-fg transition-colors duration-100 ' +
  'aria-invalid:border-danger has-aria-invalid:border-danger ' +
  'has-disabled:opacity-45 has-disabled:pointer-events-none disabled:opacity-45 disabled:pointer-events-none'

const fieldText = 'placeholder:text-fg-subtle outline-none'

export interface InputProps extends Omit<ComponentProps<'input'>, 'size'> {
  /** Icon or text before the value, e.g. a search icon. */
  leading?: ReactNode
  /** Element after the value, e.g. a unit or a clear button. */
  trailing?: ReactNode
  size?: 'sm' | 'md' | 'lg'
  wrapperClassName?: string
}

const heights = { sm: 'h-control-sm text-sm', md: 'h-control', lg: 'h-control-lg' }

export function Input({
  leading,
  trailing,
  size = 'md',
  className,
  wrapperClassName,
  ...props
}: InputProps) {
  const fieldProps = useFieldControl(props)

  if (!leading && !trailing) {
    return (
      <input
        data-slot="input"
        className={cn(
          frame,
          fieldText,
          heights[size],
          'w-full min-w-0 px-2.5 focus-visible:focus-ring-inset',
          className
        )}
        {...props}
        {...fieldProps}
      />
    )
  }

  return (
    <div
      data-slot="input"
      className={cn(
        frame,
        heights[size],
        'flex w-full min-w-0 items-center gap-1.5 px-2 has-focus-visible:focus-ring-inset',
        "[&_svg:not([class*='size-'])]:size-4 [&>svg]:shrink-0 [&>svg]:text-fg-subtle",
        wrapperClassName
      )}
    >
      {leading}
      <input
        className={cn(fieldText, 'h-full min-w-0 flex-1 bg-transparent', className)}
        {...props}
        {...fieldProps}
      />
      {trailing}
    </div>
  )
}

export function Textarea({ className, ...props }: ComponentProps<'textarea'>) {
  const fieldProps = useFieldControl(props)
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        frame,
        fieldText,
        'field-sizing-content min-h-18 w-full resize-none px-2.5 py-1.5 focus-visible:focus-ring-inset',
        className
      )}
      {...props}
      {...fieldProps}
    />
  )
}
