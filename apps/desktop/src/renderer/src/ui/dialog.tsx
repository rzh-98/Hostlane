import type { ComponentProps, ReactNode } from 'react'
import { AlertDialog, Dialog as DialogPrimitive } from 'radix-ui'
import { X } from 'lucide-react'
import { cn } from '@/lib/cn'
import { Button, buttonClass } from './button'

/*
 * Modal dialog. Focus moves into the dialog and is trapped there; Esc closes;
 * focus returns to the trigger. Wrap fields in a <form> so Enter submits.
 */

export const Dialog = DialogPrimitive.Root
export const DialogTrigger = DialogPrimitive.Trigger
export const DialogClose = DialogPrimitive.Close

const overlayClass =
  'fixed inset-0 z-50 bg-overlay data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out'

const contentClass =
  'fixed top-1/2 left-1/2 z-50 flex max-h-[calc(100vh-64px)] w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 flex-col ' +
  'rounded-lg border border-border bg-bg-raised text-fg shadow-dialog outline-none ' +
  'data-[state=open]:animate-dialog-in data-[state=closed]:animate-fade-out'

const widths = { sm: 'max-w-sm', md: 'max-w-md', lg: 'max-w-xl' } as const

export interface DialogContentProps extends ComponentProps<typeof DialogPrimitive.Content> {
  size?: keyof typeof widths
  /** Show the × button in the corner. */
  closable?: boolean
}

export function DialogContent({
  size = 'md',
  closable = true,
  className,
  children,
  ...props
}: DialogContentProps) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className={overlayClass} />
      <DialogPrimitive.Content className={cn(contentClass, widths[size], className)} {...props}>
        {children}
        {closable && (
          <DialogPrimitive.Close
            aria-label="关闭"
            className={cn(buttonClass('ghost', 'icon-sm'), 'absolute top-3 right-3')}
          >
            <X />
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  )
}

export function DialogHeader({ className, ...props }: ComponentProps<'div'>) {
  return <div className={cn('flex flex-col gap-1 px-5 pt-4 pr-12 pb-2', className)} {...props} />
}

export function DialogTitle({ className, ...props }: ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title className={cn('text-md font-semibold text-fg', className)} {...props} />
  )
}

export function DialogDescription({
  className,
  ...props
}: ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description className={cn('text-base text-fg-muted', className)} {...props} />
  )
}

export function DialogBody({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      className={cn('flex min-h-0 flex-col gap-4 overflow-y-auto px-5 py-3', className)}
      {...props}
    />
  )
}

export function DialogFooter({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      className={cn('flex items-center justify-end gap-2 px-5 pt-3 pb-4', className)}
      {...props}
    />
  )
}

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  description?: ReactNode
  confirmLabel?: string
  cancelLabel?: string
  /** Red confirm button for irreversible actions. */
  danger?: boolean
  onConfirm: () => void
  /** Where focus goes after closing; call event.preventDefault() to move it yourself. */
  onCloseAutoFocus?: (event: Event) => void
}

/**
 * Confirmation for consequential actions. Focus starts on Cancel so a stray Enter
 * never confirms a destructive action; clicking outside does not dismiss it.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel = '确定',
  cancelLabel = '取消',
  danger,
  onConfirm,
  onCloseAutoFocus
}: ConfirmDialogProps) {
  return (
    <AlertDialog.Root open={open} onOpenChange={onOpenChange}>
      <AlertDialog.Portal>
        <AlertDialog.Overlay className={overlayClass} />
        <AlertDialog.Content
          className={cn(contentClass, widths.sm)}
          onCloseAutoFocus={onCloseAutoFocus}
        >
          <div className="flex flex-col gap-1 px-5 pt-4 pb-2">
            <AlertDialog.Title className="text-md font-semibold">{title}</AlertDialog.Title>
            {description ? (
              <AlertDialog.Description className="text-base text-fg-muted">
                {description}
              </AlertDialog.Description>
            ) : (
              <AlertDialog.Description className="sr-only">{title}</AlertDialog.Description>
            )}
          </div>
          <DialogFooter>
            <AlertDialog.Cancel asChild>
              <Button>{cancelLabel}</Button>
            </AlertDialog.Cancel>
            <AlertDialog.Action asChild>
              <Button variant={danger ? 'danger' : 'primary'} onClick={onConfirm}>
                {confirmLabel}
              </Button>
            </AlertDialog.Action>
          </DialogFooter>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  )
}
