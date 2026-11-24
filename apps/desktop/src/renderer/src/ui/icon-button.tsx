import { Button, type ButtonProps } from './button'
import { Tooltip } from './tooltip'

export interface IconButtonProps extends Omit<ButtonProps, 'size' | 'children'> {
  /** Accessible name, also shown as tooltip. */
  label: string
  icon: React.ReactNode
  shortcut?: string
  size?: 'sm' | 'md'
  tooltipSide?: 'top' | 'right' | 'bottom' | 'left'
}

/** Square icon-only button with an accessible label and tooltip. */
export function IconButton({
  label,
  icon,
  shortcut,
  size = 'md',
  variant = 'ghost',
  tooltipSide,
  ...props
}: IconButtonProps) {
  return (
    <Tooltip content={label} shortcut={shortcut} side={tooltipSide}>
      <Button
        aria-label={label}
        variant={variant}
        size={size === 'sm' ? 'icon-sm' : 'icon'}
        {...props}
      >
        {icon}
      </Button>
    </Tooltip>
  )
}
