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
  Input,
  Select
} from '@/ui'
import type { SecretKind } from '@shared/protocol/SecretKind'

const DEFAULT_KEY = 'default'

export interface SecretTarget {
  host: string
  kind: SecretKind
  /** Key files of the host, offered for a passphrase. */
  keyFiles: string[]
}

interface SecretDialogProps {
  target: SecretTarget | null
  busy: boolean
  error: string | null
  onOpenChange: (open: boolean) => void
  /** The secret goes straight to the daemon, which keeps it in the system keychain. */
  onSubmit: (secret: string, keyPath: string | undefined) => void
}

/** H2: a login password or key passphrase, saved in the system keychain by the daemon. */
export function SecretDialog({ target, busy, error, onOpenChange, onSubmit }: SecretDialogProps) {
  const [secret, setSecret] = useState('')
  const [keyPath, setKeyPath] = useState(DEFAULT_KEY)

  useEffect(() => {
    setSecret('')
    setKeyPath(DEFAULT_KEY)
  }, [target])

  const password = target?.kind === 'password'

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>
            {password ? `保存 ${target?.host} 的登录密码` : `保存 ${target?.host} 的密钥口令`}
          </DialogTitle>
          <DialogDescription>
            {password
              ? '保存在系统钥匙串中，只有守护进程会读取。ssh-agent 和密钥都无法登录时才会使用密码。'
              : '保存在系统钥匙串中，用于解开加密的私钥。保存前会先验证口令能否解开密钥。'}
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            if (!secret) return
            onSubmit(secret, keyPath === DEFAULT_KEY ? undefined : keyPath)
          }}
        >
          <DialogBody>
            {!password && target && target.keyFiles.length > 1 && (
              <Field label="密钥文件">
                <Select
                  value={keyPath}
                  onValueChange={setKeyPath}
                  options={[
                    { value: DEFAULT_KEY, label: '第一个加密的密钥' },
                    ...target.keyFiles.map((file) => ({ value: file, label: file }))
                  ]}
                />
              </Field>
            )}
            <Field label={password ? '密码' : '口令'} error={error ?? undefined}>
              <Input
                type="password"
                autoFocus
                autoComplete="off"
                value={secret}
                onChange={(e) => setSecret(e.target.value)}
              />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="primary" disabled={!secret || busy}>
              保存
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
