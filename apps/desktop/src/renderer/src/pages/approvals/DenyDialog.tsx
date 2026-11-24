import { useEffect, useState } from 'react'
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Field,
  Input
} from '@/ui'
import type { Approval } from '@shared/protocol/Approval'

interface DenyDialogProps {
  /** The dialog is open while it is set. */
  approval: Approval | null
  onOpenChange: (open: boolean) => void
  onDeny: (note: string) => void
}

/** Refuses a call; the note is passed back to the agent with the denial. */
export function DenyDialog({ approval, onOpenChange, onDeny }: DenyDialogProps) {
  const [note, setNote] = useState('')
  useEffect(() => {
    if (approval) setNote('')
  }, [approval])

  return (
    <Dialog open={approval !== null} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>拒绝这次调用？</DialogTitle>
          <DialogDescription className="break-all">{approval?.summary}</DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            onDeny(note.trim())
          }}
        >
          <DialogBody>
            <Field label="原因" description="会返回给 agent，帮助它换一种做法。可以不填。">
              <Input
                autoFocus
                maxLength={500}
                placeholder="例如：先在 staging 上试"
                value={note}
                onChange={(e) => setNote(e.target.value)}
              />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="danger">
              拒绝
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
