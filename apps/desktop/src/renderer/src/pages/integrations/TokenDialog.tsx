import { useEffect, useState } from 'react'
import { Check, Copy } from 'lucide-react'
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
  IconButton,
  Input,
  SegmentedControl
} from '@/ui'
import type { ClientCreated } from '@shared/protocol/ClientCreated'
import { mcpSnippet, type McpClient } from './snippets'

interface TokenDialogProps {
  open: boolean
  command: string | null
  env: Record<string, string>
  onOpenChange: (open: boolean) => void
  /** Returns the new client, or an error message. */
  onCreate: (name: string) => Promise<ClientCreated | string>
}

const presets: { value: McpClient; label: string; name: string }[] = [
  { value: 'codex', label: 'Codex', name: 'codex' },
  { value: 'claude', label: 'Claude Code', name: 'claude-code' },
  { value: 'cursor', label: 'Cursor', name: 'cursor' }
]

/** Registers an agent and shows its token once, with an MCP config that includes it. */
export function TokenDialog({ open, command, env, onOpenChange, onCreate }: TokenDialogProps) {
  const [name, setName] = useState('')
  const [target, setTarget] = useState<McpClient>('codex')
  const [created, setCreated] = useState<ClientCreated | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (open) {
      setName('')
      setCreated(null)
      setError(null)
    }
  }, [open])

  const submit = async () => {
    setBusy(true)
    const result = await onCreate(name.trim())
    setBusy(false)
    if (typeof result === 'string') setError(result)
    else setCreated(result)
  }

  const snippet =
    created && command
      ? mcpSnippet(target, command, { ...env, HOSTLANE_TOKEN: created.token }).text
      : null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size={created ? 'lg' : 'sm'}>
        <DialogHeader>
          <DialogTitle>{created ? `${created.client.name} 的令牌` : '新建客户端令牌'}</DialogTitle>
          <DialogDescription>
            {created
              ? '令牌只显示这一次，Hostlane 只保存它的哈希。关闭前复制到 agent 的配置中。'
              : '名称会出现在审计和会话页中，例如 claude-code。'}
          </DialogDescription>
        </DialogHeader>
        {created ? (
          <>
            <DialogBody>
              <Field label="令牌">
                <div className="flex items-center gap-2">
                  <code className="selectable min-w-0 flex-1 truncate rounded-md border border-border bg-bg-subtle px-2 py-1.5 font-mono text-sm text-fg">
                    {created.token}
                  </code>
                  <CopyButton text={created.token} />
                </div>
              </Field>
              {snippet && (
                <Field label="带令牌的 MCP 配置">
                  <div className="flex flex-col gap-2">
                    <div className="flex items-center gap-2">
                      <SegmentedControl<McpClient>
                        aria-label="客户端"
                        value={target}
                        onValueChange={setTarget}
                        options={presets}
                      />
                      <span className="flex-1" />
                      <CopyButton text={snippet} />
                    </div>
                    <pre className="selectable overflow-x-auto rounded-md border border-border bg-bg-subtle p-3 font-mono text-sm text-fg-muted">
                      {snippet}
                    </pre>
                  </div>
                </Field>
              )}
              <p className="text-sm text-fg-muted">
                通过 CLI 接入时，在 agent 的环境中设置 HOSTLANE_TOKEN=令牌。
              </p>
            </DialogBody>
            <DialogFooter>
              <Button variant="primary" onClick={() => onOpenChange(false)}>
                完成
              </Button>
            </DialogFooter>
          </>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault()
              if (name.trim()) void submit()
            }}
          >
            <DialogBody>
              <Field label="名称" error={error ?? undefined}>
                <Input
                  autoFocus
                  value={name}
                  placeholder="claude-code"
                  onChange={(e) => {
                    setName(e.target.value)
                    setError(null)
                  }}
                />
              </Field>
              <div className="flex flex-wrap gap-2">
                {presets.map((preset) => (
                  <Button
                    key={preset.value}
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      setName(preset.name)
                      setTarget(preset.value)
                    }}
                  >
                    {preset.name}
                  </Button>
                ))}
              </div>
            </DialogBody>
            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                取消
              </Button>
              <Button type="submit" variant="primary" disabled={!name.trim() || busy}>
                创建
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <IconButton
      label={copied ? '已复制' : '复制'}
      icon={copied ? <Check /> : <Copy />}
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1500)
        })
      }}
    />
  )
}
