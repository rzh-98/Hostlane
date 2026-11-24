import { Check, Copy, ExternalLink } from 'lucide-react'
import { useState } from 'react'
import { formatAgo, formatBytes } from '@/app/format'
import { cn } from '@/lib/cn'
import { Badge, Button, IconButton, Section } from '@/ui'
import type { Tunnel } from '@shared/protocol/Tunnel'
import {
  browserUrl,
  expiryLabel,
  isRunning,
  localLabel,
  remoteLabel,
  stateLabels,
  tunnelTitle
} from './describe'

interface TunnelDetailProps {
  tunnel: Tunnel
  busy: boolean
  now: number
  onToggle: () => void
  onEdit: () => void
  onSave: () => void
  onDelete: () => void
}

export function TunnelDetail({
  tunnel,
  busy,
  now,
  onToggle,
  onEdit,
  onSave,
  onDelete
}: TunnelDetailProps) {
  const [copied, setCopied] = useState(false)
  const running = isRunning(tunnel)
  const url = browserUrl(tunnel)
  const state = stateLabels[tunnel.state]
  const lifetime = tunnel.saved
    ? tunnel.auto_start
      ? '已保存，随守护进程自动建立'
      : '已保存，需要手动启动'
    : tunnel.expires_at
      ? expiryLabel(tunnel.expires_at, now)
      : '守护进程退出时关闭'

  const copy = () => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <h2 className="selectable truncate text-xl font-semibold">{tunnelTitle(tunnel)}</h2>
          <div className="flex items-center gap-2">
            <Badge tone={state.tone}>{state.label}</Badge>
            {tunnel.saved ? <Badge>已保存</Badge> : <Badge>临时</Badge>}
            {tunnel.bind !== '127.0.0.1' && tunnel.bind !== '::1' && (
              <Badge tone="warning">其他设备可访问</Badge>
            )}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={tunnel.state !== 'active'} onClick={() => window.open(url)}>
            <ExternalLink />
            在浏览器中打开
          </Button>
          <IconButton
            label={copied ? '已复制' : '复制地址'}
            icon={copied ? <Check /> : <Copy />}
            variant="secondary"
            onClick={copy}
          />
          <Button disabled={busy} onClick={onToggle}>
            {running ? '停止' : '启动'}
          </Button>
          <Button disabled={busy} onClick={onEdit}>
            编辑
          </Button>
          {!tunnel.saved && (
            <Button disabled={busy} onClick={onSave}>
              保存
            </Button>
          )}
          <Button variant="ghost" disabled={busy} onClick={onDelete}>
            删除
          </Button>
        </div>
      </header>

      <Section title="地址" divided>
        <Row label="本机地址" value={url} mono />
        <Row label="本机绑定" value={localLabel(tunnel)} mono />
        <Row label="远程" value={remoteLabel(tunnel)} mono />
      </Section>

      <Section title="状态" divided>
        <Row label="状态" value={state.label} />
        {tunnel.error && (
          <Row
            label={tunnel.state === 'reconnecting' ? '重连原因' : '错误'}
            value={[tunnel.error.message, tunnel.error.data.hint].filter(Boolean).join('\n')}
            tone="danger"
          />
        )}
        {tunnel.last_connection_error && (
          <Row label="最近一次转发失败" value={tunnel.last_connection_error} tone="danger" />
        )}
        <Row
          label="连接"
          value={`${tunnel.active_connections} 个活跃 · 累计 ${tunnel.total_connections} 个`}
        />
        <Row
          label="流量"
          value={`↓ ${formatBytes(tunnel.bytes_in)} · ↑ ${formatBytes(tunnel.bytes_out)}`}
        />
      </Section>

      <Section title="来源" divided>
        <Row
          label="创建者"
          value={[tunnel.created_by.name, tunnel.created_by.session].filter(Boolean).join(' · ')}
        />
        <Row label="创建于" value={formatAgo(tunnel.created_at, now)} />
        <Row label="生命周期" value={lifetime} />
        <Row label="ID" value={tunnel.id} mono />
      </Section>
    </div>
  )
}

function Row({
  label,
  value,
  mono,
  tone
}: {
  label: string
  value: string
  mono?: boolean
  tone?: 'danger'
}) {
  return (
    <div className="flex min-h-row items-start justify-between gap-6 py-2.5">
      <span className="shrink-0 text-base text-fg">{label}</span>
      <span
        className={cn(
          'selectable min-w-0 text-right text-sm break-all whitespace-pre-line',
          tone === 'danger' ? 'text-danger-text' : 'text-fg-muted',
          mono && 'font-mono'
        )}
      >
        {value}
      </span>
    </div>
  )
}
