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
import type { Host } from '@shared/protocol/Host'
import type { HostEnv } from '@shared/protocol/HostEnv'

const NONE = 'none'

interface LabelDialogProps {
  /** The dialog is open while it is set. */
  host: Host | null
  busy: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (env: HostEnv | undefined, tags: string[]) => void
}

/** H4: environment and tags of a host, used by policy rules and defaults. */
export function LabelDialog({ host, busy, onOpenChange, onSubmit }: LabelDialogProps) {
  const [env, setEnv] = useState<string>(NONE)
  const [tags, setTags] = useState('')

  useEffect(() => {
    if (!host) return
    setEnv(host.env ?? NONE)
    setTags(host.tags.join(', '))
  }, [host])

  return (
    <Dialog open={host !== null} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>编辑 {host?.alias} 的标签</DialogTitle>
          <DialogDescription>
            prod
            主机默认更严格：用户态修改需要审批，破坏性操作直接拒绝。环境和标签都清空后，恢复使用
            ~/.ssh/config 中的 # hostlane: 注释。
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            onSubmit(
              env === NONE ? undefined : (env as HostEnv),
              tags
                .split(/[,，\s]+/)
                .map((tag) => tag.trim())
                .filter(Boolean)
            )
          }}
        >
          <DialogBody>
            <Field label="环境">
              <Select
                value={env}
                onValueChange={setEnv}
                options={[
                  { value: NONE, label: '未标记' },
                  { value: 'dev', label: 'dev' },
                  { value: 'staging', label: 'staging' },
                  { value: 'prod', label: 'prod' }
                ]}
              />
            </Field>
            <Field label="标签" description="用逗号或空格分隔">
              <Input
                placeholder="例如 web, eu"
                value={tags}
                onChange={(e) => setTags(e.target.value)}
              />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="primary" disabled={busy}>
              保存
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
