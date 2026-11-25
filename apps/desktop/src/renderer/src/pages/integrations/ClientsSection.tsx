import { useEffect, useState } from 'react'
import { Plus, Trash2 } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { formatAgo } from '@/app/format'
import { Button, ConfirmDialog, FormRow, IconButton, Section, Switch } from '@/ui'
import type { ClientCreated } from '@shared/protocol/ClientCreated'
import type { ClientsList } from '@shared/protocol/ClientsList'
import { TokenDialog } from './TokenDialog'

interface ClientsSectionProps {
  command: string | null
  env: Record<string, string>
  onNotice: (notice: { tone: 'success' | 'danger'; text: string }) => void
}

/** A5: registered agents with their own token, so the audit log shows who called. */
export function ClientsSection({ command, env, onNotice }: ClientsSectionProps) {
  const online = daemonStore.use().phase === 'online'
  const [list, setList] = useState<ClientsList | null>(null)
  const [creating, setCreating] = useState(false)
  const [revoking, setRevoking] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const load = async () => {
    const reply = await callHostlane<ClientsList>('clients.list')
    if (reply.ok) setList(reply.result)
  }

  useEffect(() => {
    if (online) void load()
  }, [online])

  const configure = async (require: boolean) => {
    setBusy(true)
    const reply = await callHostlane<ClientsList>('clients.configure', { require_token: require })
    setBusy(false)
    if (reply.ok) setList(reply.result)
    else onNotice({ tone: 'danger', text: `修改失败：${reply.error.message}` })
  }

  const create = async (name: string): Promise<ClientCreated | string> => {
    const reply = await callHostlane<ClientCreated>('clients.create', { name })
    if (!reply.ok) return reply.error.message
    void load()
    return reply.result
  }

  const revoke = async (name: string) => {
    const reply = await callHostlane('clients.revoke', { name })
    if (reply.ok) {
      onNotice({ tone: 'success', text: `已吊销 ${name} 的令牌。` })
      void load()
    } else {
      onNotice({ tone: 'danger', text: `吊销失败：${reply.error.message}` })
    }
  }

  return (
    <Section
      title="客户端令牌"
      description="给每个 agent 发一个令牌（环境变量 HOSTLANE_TOKEN）。带令牌的调用在审计中显示登记的名称并标记为已验证，不能冒用别的名称。"
      actions={
        <Button disabled={!online} onClick={() => setCreating(true)}>
          <Plus />
          新建令牌
        </Button>
      }
      divided
    >
      <FormRow
        label="要求 agent 提供令牌"
        description="开启后，没有有效令牌的 agent 调用会被拒绝（UNAUTHORIZED）。桌面端和手动运行的 CLI 不受影响。"
      >
        <Switch
          aria-label="要求 agent 提供令牌"
          checked={list?.require_token ?? false}
          disabled={!list || busy}
          onCheckedChange={(checked) => void configure(checked)}
        />
      </FormRow>
      {list?.clients.map((client) => (
        <div key={client.name} className="flex items-center gap-3 py-2">
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-mono text-sm text-fg">{client.name}</span>
            <span className="text-sm text-fg-muted">
              创建于 {formatAgo(client.created_at)} ·{' '}
              {client.last_used_at ? `最近使用 ${formatAgo(client.last_used_at)}` : '尚未使用'}
            </span>
          </div>
          <IconButton
            label={`吊销 ${client.name}`}
            icon={<Trash2 />}
            onClick={() => setRevoking(client.name)}
          />
        </div>
      ))}
      {list && list.clients.length === 0 && (
        <p className="py-2 text-sm text-fg-muted">还没有登记的 agent。</p>
      )}
      <TokenDialog
        open={creating}
        command={command}
        env={env}
        onOpenChange={setCreating}
        onCreate={create}
      />
      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(open) => !open && setRevoking(null)}
        title={`吊销 ${revoking ?? ''} 的令牌？`}
        description="使用这个令牌的 agent 会立即收到 UNAUTHORIZED，需要新建令牌并更新它的配置。"
        confirmLabel="吊销"
        danger
        onConfirm={() => {
          if (revoking) void revoke(revoking)
          setRevoking(null)
        }}
      />
    </Section>
  )
}
