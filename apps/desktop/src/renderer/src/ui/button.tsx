import type { ComponentProps } from 'react'
import { Slot } from 'radix-ui'
import { cn } from '@/lib/cn'

const base =
  'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md font-medium select-none ' +
  'transition-colors duration-100 outline-none focus-visible:focus-ring ' +
  'disabled:pointer-events-none disabled:opacity-45 aria-disabled:pointer-events-none aria-disabled:opacity-45 ' +
  "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4"

const variants = {
  /** The one main action of a view. */
  primary: 'bg-accent text-accent-fg hover:bg-accent-hover active:bg-accent-active',
  /** Default bordered button. */
  secondary:
    'border border-border-strong bg-control text-fg hover:bg-control-hover active:bg-control-active',
  /** Quiet button for toolbars and inline actions. Shows pressed state via aria-pressed / data-state=on. */
  ghost:
    'text-fg-muted hover:bg-hover hover:text-fg active:bg-active aria-expanded:bg-active aria-expanded:text-fg ' +
    'aria-pressed:bg-accent-subtle aria-pressed:text-accent-text data-[state=on]:bg-accent-subtle data-[state=on]:text-accent-text',
  /** Destructive confirmation. */
  danger: 'bg-danger text-white hover:bg-danger-hover active:bg-danger-hover'
} as const

const sizes = {
  sm: 'h-control-sm px-2 text-sm',
  md: 'h-control px-3 text-base',
  lg: 'h-control-lg px-4 text-base',
  icon: 'size-control px-0',
  'icon-sm': 'size-control-sm px-0 [&_svg:not([class*=size-])]:size-3.5'
} as const

export type ButtonVariant = keyof typeof variants
export type ButtonSize = keyof typeof sizes

export interface ButtonProps extends ComponentProps<'button'> {
  variant?: ButtonVariant
  size?: ButtonSize
  /** Render the child element (e.g. a link) with button styles. */
  asChild?: boolean
}

export function buttonClass(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md'): string {
  return cn(base, variants[variant], sizes[size])
}

export function Button({
  variant = 'secondary',
  size = 'md',
  asChild,
  className,
  type = 'button',
  ...props
}: ButtonProps) {
  const Comp = asChild ? Slot.Root : 'button'
  return (
    <Comp
      data-slot="button"
      type={asChild ? undefined : type}
      className={cn(buttonClass(variant, size), className)}
      {...props}
    />
  )
}
