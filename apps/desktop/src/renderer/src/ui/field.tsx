import { createContext, useContext, useId, type ComponentProps, type ReactNode } from 'react'
import { cn } from '@/lib/cn'

interface FieldContextValue {
  id: string
  descriptionId?: string
  errorId?: string
  invalid: boolean
}

const FieldContext = createContext<FieldContextValue | null>(null)

/**
 * Props a form control should spread to get wired to its surrounding <Field>:
 * id (for the label), aria-describedby and aria-invalid.
 */
export function useFieldControl(props: { id?: string; 'aria-describedby'?: string }) {
  const field = useContext(FieldContext)
  if (!field) return {}
  const describedBy = [props['aria-describedby'], field.descriptionId, field.errorId]
    .filter(Boolean)
    .join(' ')
  return {
    id: props.id ?? field.id,
    'aria-describedby': describedBy || undefined,
    'aria-invalid': field.invalid || undefined
  }
}

export function Label({ className, ...props }: ComponentProps<'label'>) {
  return <label className={cn('text-base font-medium text-fg', className)} {...props} />
}

export interface FieldProps {
  label?: ReactNode
  description?: ReactNode
  /** Error message; also marks the control as invalid. */
  error?: ReactNode
  className?: string
  children: ReactNode
}

/** Stacked label + control + help text, for forms and dialogs. */
export function Field({ label, description, error, className, children }: FieldProps) {
  const id = useId()
  const value: FieldContextValue = {
    id,
    descriptionId: description ? `${id}-description` : undefined,
    errorId: error ? `${id}-error` : undefined,
    invalid: Boolean(error)
  }
  return (
    <FieldContext.Provider value={value}>
      <div className={cn('flex flex-col gap-1.5', className)}>
        {label && <Label htmlFor={id}>{label}</Label>}
        {children}
        {description && !error && (
          <p id={value.descriptionId} className="text-sm text-fg-muted">
            {description}
          </p>
        )}
        {error && (
          <p id={value.errorId} role="alert" className="text-sm text-danger-text">
            {error}
          </p>
        )}
      </div>
    </FieldContext.Provider>
  )
}

export interface FormRowProps {
  label: ReactNode
  description?: ReactNode
  /** The control, aligned to the trailing edge. */
  children: ReactNode
  className?: string
}

/**
 * Horizontal "label on the left, control on the right" row used by settings and
 * property panels. Rows are separated by hairlines when stacked inside <Section>.
 */
export function FormRow({ label, description, children, className }: FormRowProps) {
  const id = useId()
  return (
    <FieldContext.Provider
      value={{ id, descriptionId: description ? `${id}-d` : undefined, invalid: false }}
    >
      <div className={cn('flex min-h-row items-center justify-between gap-6 py-2.5', className)}>
        <div className="flex min-w-0 flex-col gap-0.5">
          <Label htmlFor={id} className="font-normal">
            {label}
          </Label>
          {description && (
            <p id={`${id}-d`} className="text-sm text-fg-muted">
              {description}
            </p>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">{children}</div>
      </div>
    </FieldContext.Provider>
  )
}
