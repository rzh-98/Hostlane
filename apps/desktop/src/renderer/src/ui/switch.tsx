import type { ComponentProps } from 'react'
import { Switch as SwitchPrimitive } from 'radix-ui'
import { cn } from '@/lib/cn'
import { useFieldControl } from './field'

export function Switch({ className, ...props }: ComponentProps<typeof SwitchPrimitive.Root>) {
  const fieldProps = useFieldControl(props)
  return (
    <SwitchPrimitive.Root
      className={cn(
        'peer inline-flex h-5 w-8.5 shrink-0 items-center rounded-full p-0.5 transition-colors duration-150 outline-none',
        'bg-border-strong data-[state=checked]:bg-accent focus-visible:focus-ring',
        'disabled:pointer-events-none disabled:opacity-45',
        className
      )}
      {...props}
      {...fieldProps}
    >
      <SwitchPrimitive.Thumb className="block size-4 rounded-full bg-white shadow-[0_1px_2px_oklch(0_0_0/0.25)] transition-transform duration-150 ease-out data-[state=checked]:translate-x-3.5" />
    </SwitchPrimitive.Root>
  )
}
