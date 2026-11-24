import { useEffect, useState } from 'react'
import { TriangleAlert, Undo2 } from 'lucide-react'
import { cn } from '@/lib/cn'
import {
  Button,
  ConfirmDialog,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Notice
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { FileChange } from '@shared/protocol/FileChange'
import type { FileChangeResult } from '@shared/protocol/FileChangeResult'
import type { FileOperation } from '@shared/protocol/FileOperation'
import { callHostlane } from './daemon'
import { formatBytes, formatTime } from './format'

const operationLabels: Record<FileOperation, string> = {
  write: '写入',
  patch: '修改',
  upload: '上传',
  revert: '撤销'
}

interface DiffDialogProps {
  /** The dialog is open while it is set. */
  changeId: string | null
  onClose: () => void
}

/** A recorded file change: its diff, and a way to undo it. */
export function DiffDialog({ changeId, onClose }: DiffDialogProps) {
  return (
    <Dialog open={changeId !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent size="lg" className="h-[calc(100vh-96px)]">
        {changeId && <DiffBody key={changeId} changeId={changeId} />}
      </DialogContent>
    </Dialog>
  )
}

function lineClass(line: string): string {
  if (line.startsWith('+++') || line.startsWith('---')) return 'font-semibold text-fg'
  if (line.startsWith('@@')) return 'text-accent-text'
  if (line.startsWith('+')) return 'bg-success-subtle text-success-text'
  if (line.startsWith('-')) return 'bg-danger-subtle text-danger-text'
  return 'text-fg-muted'
}

function DiffBody({ changeId }: { changeId: string }) {
  const [change, setChange] = useState<FileChange | null>(null)
  const [error, setError] = useState<HostlaneError | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [reverting, setReverting] = useState(false)
  const [reverted, setReverted] = useState<FileChangeResult | null>(null)

  useEffect(() => {
    let current = true
    void callHostlane<FileChange>('files.change', { change_id: changeId }).then((reply) => {
      if (!current) return
      if (reply.ok) setChange(reply.result)
      else setError(reply.error)
    })
    return () => {
      current = false
    }
  }, [changeId])

  async function revert() {
    setReverting(true)
    const reply = await callHostlane<FileChangeResult>('files.revert', { change_id: changeId })
    setReverting(false)
    if (reply.ok) {
      setError(null)
      setReverted(reply.result)
    } else {
      setError(reply.error)
    }
  }

  const canRevert = change !== null && (change.created || change.has_backup)
  const lines = change?.diff?.split('\n') ?? []
  if (lines[lines.length - 1] === '') lines.pop()

  return (
    <>
      <DialogHeader>
        <DialogTitle className="truncate font-mono">{change?.path ?? changeId}</DialogTitle>
        <DialogDescription className="text-sm">
          {change
            ? [
                change.host,
                operationLabels[change.operation],
                change.client.name,
                formatTime(change.created_at),
                change.deleted ? '已删除文件' : `+${change.added_lines} −${change.removed_lines}`
              ].join(' · ')
            : changeId}
        </DialogDescription>
      </DialogHeader>
      {error && (
        <Notice tone="danger" icon={<TriangleAlert />} className="border-t">
          {error.message}
          {error.hint && <span className="block text-fg-muted">{error.hint}</span>}
        </Notice>
      )}
      {reverted && (
        <Notice tone="success" className="border-t">
          {reverted.deleted ? '已删除该文件' : '已恢复为修改前的内容'}（{reverted.change_id}）
        </Notice>
      )}
      <DialogBody className="min-h-0 flex-1">
        <pre className="selectable min-h-0 flex-1 overflow-auto rounded-md border border-border bg-bg-subtle py-2 font-mono text-sm">
          {!change ? (
            error ? null : (
              <span className="px-3 text-fg-muted">读取中…</span>
            )
          ) : change.diff ? (
            lines.map((line, index) => (
              <div
                key={index}
                className={cn('px-3 whitespace-pre-wrap break-all', lineClass(line))}
              >
                {line || ' '}
              </div>
            ))
          ) : (
            <span className="px-3 text-fg-muted">
              没有保存 diff（二进制或过大的文件，{formatBytes(change.size)}）
            </span>
          )}
        </pre>
      </DialogBody>
      <DialogFooter className="justify-between">
        <span className="font-mono text-sm text-fg-muted">{changeId}</span>
        {canRevert && !reverted && (
          <Button disabled={reverting} onClick={() => setConfirming(true)}>
            <Undo2 />
            撤销此修改
          </Button>
        )}
      </DialogFooter>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="撤销这次修改？"
        description={
          change?.created
            ? `将删除 ${change.host} 上的 ${change.path}。`
            : `将把 ${change?.host} 上的 ${change?.path} 恢复为修改前的内容。文件在此之后又被改过时，撤销会失败。`
        }
        confirmLabel="撤销"
        danger={change?.created}
        onConfirm={() => void revert()}
      />
    </>
  )
}
