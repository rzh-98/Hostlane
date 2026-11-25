import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Select
} from '@/ui'
import { useState } from 'react'
import type { SecretKind } from '@shared/protocol/SecretKind'

export interface SecretTarget {
  host: string
  kind: SecretKind
  keys: string[]
}

export function SecretDialog({
  target,
  onClose,
  onSaved
}: {
  target: SecretTarget | null
  onClose: () => void
  onSaved: () => void
}) {
  const host = target?.host ?? ''
  const [key, setKey] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const password = target?.kind === 'password'
  const save = async () => {
    if (!target || busy || !window.desktop) return
    setBusy(true)
    setError('')
    try {
      const reply = await window.desktop.hostlane.promptSecret(host, target.kind, key || undefined)
      if (!reply.ok) setError(reply.error.message)
      else if (reply.result) onSaved()
    } catch {
      setError('无法打开系统密码窗口。')
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open={target !== null} onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>
            为 {host} 保存{password ? '登录密码' : '密钥口令'}
          </DialogTitle>
          <DialogDescription>
            在系统密码窗口中输入，保存到本机钥匙串。Hostlane 页面不会接触密码。
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          {!password && (
            <Select
              aria-label="密钥文件"
              disabled={busy}
              value={key || '*'}
              onValueChange={(value) => setKey(value === '*' ? '' : value)}
              options={[
                { value: '*', label: '第一个加密的密钥' },
                ...(target?.keys ?? []).map((file) => ({ value: file, label: file }))
              ]}
            />
          )}
          {error && (
            <p role="alert" className="text-sm text-danger-text">
              {error}
            </p>
          )}
        </DialogBody>
        <DialogFooter>
          <Button disabled={busy} onClick={onClose}>
            关闭
          </Button>
          <Button disabled={busy || !window.desktop} variant="primary" onClick={() => void save()}>
            {busy ? '等待系统窗口…' : '打开安全输入窗口'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
