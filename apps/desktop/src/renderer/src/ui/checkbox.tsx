import { useId, type ComponentProps, type ReactNode } from 'react'
import { Checkbox as CheckboxPrimitive } from 'radix-ui'
import { Check, Minus } from 'lucide-react'
import { cn } from '@/lib/cn'

export interface CheckboxProps extends ComponentProps<typeof CheckboxPrimitive.Root> {
  /** Optional inline label; clicking it toggles the box. */
  label?: ReactNode
  description?: ReactNode
}

export function Checkbox({ label, description, className, id, ...props }: CheckboxProps) {
  const autoId = useId()
  const boxId = id ?? autoId
  const box = (
    <CheckboxPrimitive.Root
      id={boxId}
      className={cn(
        'peer mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-sm border border-border-strong bg-control text-accent-fg outline-none transition-colors duration-100',
        'data-[state=checked]:border-accent data-[state=checked]:bg-accent data-[state=indeterminate]:border-accent data-[state=indeterminate]:bg-accent',
        'focus-visible:focus-ring disabled:pointer-events-none disabled:opacity-45',
        !label && 'mt-0',
        className
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator>
        {props.checked === 'indeterminate' ? (
          <Minus className="size-3" strokeWidth={3} />
        ) : (
          <Check className="size-3" strokeWidth={3} />
        )}
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  )

  if (!label) return box

  return (
    <div className="flex items-start gap-2">
      {box}
      <label htmlFor={boxId} className="flex flex-col gap-0.5 peer-disabled:opacity-45">
        <span className="text-base text-fg">{label}</span>
        {description && <span className="text-sm text-fg-muted">{description}</span>}
      </label>
    </div>
  )
}
