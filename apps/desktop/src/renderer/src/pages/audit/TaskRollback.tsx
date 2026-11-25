import { useEffect, useRef, useState } from 'react'
import { callHostlane } from '@/app/daemon'
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle
} from '@/ui'
import type { RollbackItem } from '@shared/protocol/RollbackItem'
import type { Approval } from '@shared/protocol/Approval'

export function TaskRollback({
  task,
  host,
  client
}: {
  task: string
  host?: string
  client?: string
}) {
  const [items, setItems] = useState<RollbackItem[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [approval, setApproval] = useState<Approval | null>(null)
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])
  const preview = async () => {
    setMessage('')
    setBusy(true)
    const result = await callHostlane<RollbackItem[]>('files.rollback_plan', { task, host, client })
    setBusy(false)
    setItems(result.ok ? result.result : [])
    setMessage(
      result.ok ? (result.result.length ? '' : '没有待撤销的修改。') : result.error.message
    )
  }
  const apply = async () => {
    if (!items?.length || busy) return
    setBusy(true)
    setMessage('')
    let ticket = approval?.id
    try {
      if (ticket) {
        const reply = await callHostlane('approvals.resolve', { id: ticket, approve: true })
        if (!reply.ok) throw new Error(reply.error.message)
        setApproval(null)
      }
      for (const [index, item] of items.entries()) {
        if (!alive.current) return
        const result = await callHostlane('files.revert', {
          change_id: item.change_id,
          async: true,
          ticket_id: ticket
        })
        ticket = undefined
        if (!result.ok) {
          const id = result.error.details?.ticket_id
          if (result.error.kind === 'APPROVAL_REQUIRED' && typeof id === 'string') {
            const pending = await callHostlane<Approval>('approvals.get', { id })
            if (!pending.ok) throw new Error(pending.error.message)
            setApproval(pending.result)
            return
          }
          throw new Error(`${item.host} · ${item.path}：${result.error.message}`)
        }
        setItems(items.slice(index + 1))
      }
      setMessage('已撤销预览中的全部修改。')
    } catch (err) {
      setApproval(null)
      setMessage(`已停止，先前成功的项目不会重复处理。${err instanceof Error ? err.message : ''}`)
      setItems([])
    } finally {
      setBusy(false)
    }
  }
  const close = () => {
    if (busy) return
    setItems(null)
    setApproval(null)
    if (approval) void callHostlane('approvals.resolve', { id: approval.id, approve: false })
  }
  return (
    <>
      <Button disabled={!task || busy} onClick={() => void preview()}>
        预览回滚…
      </Button>
      <Dialog open={items !== null} onOpenChange={(open) => !open && close()}>
        <DialogContent size="md">
          <DialogHeader>
            <DialogTitle>回滚任务 {task}</DialogTitle>
            <DialogDescription>
              仅撤销以下修改，从新到旧逐项检查。遇到冲突即停止，不覆盖外部改动。
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            {items?.map((item) => (
              <div key={item.change_id} className="selectable break-all font-mono text-sm">
                {item.host} · {item.path}
              </div>
            ))}
            {approval && (
              <div role="alert">
                <strong>需要单独审批：{approval.risk}</strong>
                <p>{approval.summary}</p>
                <p>
                  {approval.host} · {approval.env ?? '未标记环境'} · {approval.client.name}
                </p>
                {approval.diff && (
                  <pre className="selectable overflow-auto font-mono text-sm">{approval.diff}</pre>
                )}
                <p>{approval.reasons.join('；')}</p>
              </div>
            )}
            {message && <p role="status">{message}</p>}
          </DialogBody>
          <DialogFooter>
            <Button disabled={busy} onClick={close}>
              {approval ? '拒绝并关闭' : '关闭'}
            </Button>
            {!!items?.length && (
              <Button disabled={busy} variant="danger" onClick={() => void apply()}>
                {busy ? '处理中…' : approval ? '本次允许并继续' : `确认撤销 ${items.length} 项`}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
