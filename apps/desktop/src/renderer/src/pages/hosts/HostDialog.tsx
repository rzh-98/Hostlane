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
import type { HostAddParams } from '@shared/protocol/HostAddParams'
import type { HostEnv } from '@shared/protocol/HostEnv'

const NONE = 'none'
const ID_PATTERN = /^[A-Za-z0-9._-]+$/

interface HostDialogProps {
  open: boolean
  /** Saved settings of the host being edited; null adds a new host. */
  editing: HostAddParams | null
  busy: boolean
  error: string | null
  onOpenChange: (open: boolean) => void
  onSubmit: (params: HostAddParams) => void
}

/** H1: a host kept by Hostlane, for servers that are not in ~/.ssh/config. */
export function HostDialog({
  open,
  editing,
  busy,
  error,
  onOpenChange,
  onSubmit
}: HostDialogProps) {
  const [id, setId] = useState('')
  const [hostname, setHostname] = useState('')
  const [user, setUser] = useState('')
  const [port, setPort] = useState('')
  const [jump, setJump] = useState('')
  const [key, setKey] = useState('')
  const [env, setEnv] = useState<string>(NONE)
  const [tags, setTags] = useState('')

  useEffect(() => {
    if (!open) return
    setId(editing?.id ?? '')
    setHostname(editing?.hostname ?? '')
    setUser(editing?.user ?? '')
    setPort(editing?.port && editing.port !== 22 ? String(editing.port) : '')
    setJump(editing?.proxy_jump ?? '')
    setKey(editing?.identity_file ?? '')
    setEnv(editing?.env ?? NONE)
    setTags(editing?.tags.join(', ') ?? '')
  }, [open, editing])

  const portNumber = port.trim() ? Number(port) : 22
  const portValid = Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535
  const idValid = ID_PATTERN.test(id.trim())
  const valid = idValid && hostname.trim() !== '' && portValid

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="md">
        <DialogHeader>
          <DialogTitle>{editing ? `编辑 ${editing.id}` : '添加主机'}</DialogTitle>
          <DialogDescription>
            保存在 Hostlane 中，不会修改 ~/.ssh/config。认证依次尝试
            ssh-agent、密钥文件和已保存的密码。
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            if (!valid) return
            const optional = (value: string) => value.trim() || undefined
            onSubmit({
              id: id.trim(),
              hostname: hostname.trim(),
              user: optional(user),
              port: port.trim() ? portNumber : undefined,
              proxy_jump: optional(jump),
              identity_file: optional(key),
              env: env === NONE ? undefined : (env as HostEnv),
              tags: tags
                .split(/[,，\s]+/)
                .map((tag) => tag.trim())
                .filter(Boolean)
            })
          }}
        >
          <DialogBody>
            <Field
              label="名称"
              description="在命令和 agent 中使用，例如 hostlane exec web1"
              error={id.trim() && !idValid ? '只能包含字母、数字、点、下划线和连字符' : undefined}
            >
              <Input
                autoFocus={!editing}
                disabled={editing !== null}
                placeholder="web1"
                value={id}
                onChange={(e) => setId(e.target.value)}
              />
            </Field>
            <Field label="主机地址">
              <Input
                autoFocus={editing !== null}
                placeholder="203.0.113.10 或 web1.example.com"
                value={hostname}
                onChange={(e) => setHostname(e.target.value)}
              />
            </Field>
            <div className="grid grid-cols-[1fr_8rem] gap-3">
              <Field label="用户">
                <Input
                  placeholder="默认为当前用户"
                  value={user}
                  onChange={(e) => setUser(e.target.value)}
                />
              </Field>
              <Field label="端口" error={portValid ? undefined : '1–65535'}>
                <Input
                  inputMode="numeric"
                  placeholder="22"
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                />
              </Field>
            </div>
            <Field label="跳板机" description="其他主机的名称，或 user@host:port">
              <Input placeholder="无" value={jump} onChange={(e) => setJump(e.target.value)} />
            </Field>
            <Field
              label="密钥文件"
              description="填写后只使用这个密钥；留空则使用 ssh-agent 和默认密钥"
            >
              <div className="flex items-center gap-2">
                <Input
                  placeholder="~/.ssh/id_ed25519"
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                />
                <Button
                  type="button"
                  disabled={!window.desktop}
                  onClick={async () => {
                    const path = await window.desktop?.hostlane.chooseKey()
                    if (path) setKey(path)
                  }}
                >
                  浏览…
                </Button>
              </div>
            </Field>
            <div className="grid grid-cols-[8rem_1fr] gap-3">
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
              <Field label="标签">
                <Input
                  placeholder="用逗号或空格分隔"
                  value={tags}
                  onChange={(e) => setTags(e.target.value)}
                />
              </Field>
            </div>
            {error && (
              <p role="alert" className="text-sm text-danger-text">
                {error}
              </p>
            )}
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="primary" disabled={!valid || busy}>
              {editing ? '保存' : '添加'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
