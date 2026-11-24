import { clsx, type ClassValue } from 'clsx'
import { extendTailwindMerge } from 'tailwind-merge'

/** Tells tailwind-merge about the custom token names in styles/tokens.css. */
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      spacing: [
        'control-sm',
        'control',
        'control-lg',
        'row',
        'toolbar',
        'sidebar',
        'page',
        'traffic-light'
      ],
      shadow: ['popover', 'dialog']
    }
  }
})

/** Joins class names and lets later Tailwind classes override earlier ones. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}
