import { RadioGroup } from 'radix-ui'
import { Check } from 'lucide-react'
import { cn } from '@/lib/cn'

export interface AccentOption {
  /** OKLCH hue, or null for `--brand-hue` in tokens.css. */
  hue: number | null
  label: string
}

export const accentOptions: AccentOption[] = [
  { hue: null, label: '默认' },
  { hue: 285, label: '紫' },
  { hue: 0, label: '粉' },
  { hue: 40, label: '橙' },
  { hue: 155, label: '绿' },
  { hue: 205, label: '青' }
]

interface AccentPickerProps {
  value: number | null
  onChange: (hue: number | null) => void
}

/** Swatches rendered from the same OKLCH formula as --color-accent. ←/→ to move. */
export function AccentPicker({ value, onChange }: AccentPickerProps) {
  return (
    <RadioGroup.Root
      aria-label="强调色"
      orientation="horizontal"
      value={String(value)}
      onValueChange={(next) => onChange(next === 'null' ? null : Number(next))}
      className="flex items-center gap-1.5"
    >
      {accentOptions.map((option) => (
        <RadioGroup.Item
          key={option.label}
          value={String(option.hue)}
          aria-label={option.label}
          title={option.label}
          style={
            option.hue === null
              ? undefined
              : { background: `oklch(0.58 var(--brand-chroma) ${option.hue})` }
          }
          className={cn(
            'flex size-5 items-center justify-center rounded-full text-white outline-none transition-transform duration-100',
            'hover:scale-110 focus-visible:focus-ring data-[state=checked]:ring-2 data-[state=checked]:ring-border-strong data-[state=checked]:ring-offset-1 data-[state=checked]:ring-offset-bg',
            option.hue === null && 'bg-[oklch(0.58_var(--brand-chroma)_var(--brand-hue))]'
          )}
        >
          <RadioGroup.Indicator>
            <Check className="size-3" strokeWidth={3} />
          </RadioGroup.Indicator>
        </RadioGroup.Item>
      ))}
    </RadioGroup.Root>
  )
}
