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

export interface FreePortQuery {
  from: number
  to: number
  count: number
}

interface FreePortDialogProps {
  open: boolean
  host: string
  busy: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (query: FreePortQuery) => void
}

function parsePort(text: string): number | null {
  const value = Number(text)
  return Number.isInteger(value) && value >= 1 && value <= 65535 ? value : null
}

export function FreePortDialog({ open, host, busy, onOpenChange, onSubmit }: FreePortDialogProps) {
  const [from, setFrom] = useState('8000')
  const [to, setTo] = useState('9999')
  const [count, setCount] = useState('1')

  useEffect(() => {
    if (open) setCount('1')
  }, [open])

  const fromPort = parsePort(from)
  const toPort = parsePort(to)
  const countValue = Number(count)
  const rangeError =
    fromPort === null || toPort === null
      ? '端口范围是 1–65535'
      : fromPort > toPort
        ? '起始端口不能大于结束端口'
        : undefined
  const countError =
    Number.isInteger(countValue) && countValue >= 1 && countValue <= 100 ? undefined : '1–100 个'
  const valid = !rangeError && !countError

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>查找空闲端口</DialogTitle>
          <DialogDescription>
            在 {host} 上查找没有进程监听、也没有备注预留的端口。
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            if (valid && fromPort !== null && toPort !== null) {
              onSubmit({ from: fromPort, to: toPort, count: countValue })
            }
          }}
        >
          <DialogBody>
            <div className="grid grid-cols-2 gap-3">
              <Field label="起始端口" error={rangeError}>
                <Input inputMode="numeric" value={from} onChange={(e) => setFrom(e.target.value)} />
              </Field>
              <Field label="结束端口">
                <Input inputMode="numeric" value={to} onChange={(e) => setTo(e.target.value)} />
              </Field>
            </div>
            <Field label="数量" error={countError}>
              <Input inputMode="numeric" value={count} onChange={(e) => setCount(e.target.value)} />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="primary" disabled={!valid || busy}>
              {busy ? '查找中…' : '查找'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
