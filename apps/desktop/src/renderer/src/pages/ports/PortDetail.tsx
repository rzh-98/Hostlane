import { useState } from 'react'
import { ExternalLink } from 'lucide-react'
import { formatAgo } from '@/app/format'
import { cn } from '@/lib/cn'
import { Badge, Button, Field, FormRow, Input, Section, Textarea } from '@/ui'
import type { ListeningPort } from '@shared/protocol/ListeningPort'
import {
  bindLabels,
  certLabel,
  certTone,
  ownerLabel,
  probeErrorLabel,
  serviceLabel,
  statusTone
} from './describe'

export interface NoteDraft {
  name: string
  project: string
  note: string
}

interface PortDetailProps {
  port: ListeningPort
  probing: boolean
  savingNote: boolean
  forwarding: boolean
  onForward: () => void
  onProbe: (serverName: string) => void
  onSaveNote: (draft: NoteDraft) => void
}

export function PortDetail({
  port,
  probing,
  savingNote,
  forwarding,
  onForward,
  onProbe,
  onSaveNote
}: PortDetailProps) {
  const [serverName, setServerName] = useState('')
  const [draft, setDraft] = useState<NoteDraft>({
    name: port.note?.name ?? '',
    project: port.note?.project ?? '',
    note: port.note?.note ?? ''
  })
  const saved = {
    name: port.note?.name ?? '',
    project: port.note?.project ?? '',
    note: port.note?.note ?? ''
  }
  const dirty =
    draft.name.trim() !== saved.name ||
    draft.project.trim() !== saved.project ||
    draft.note.trim() !== saved.note
  const { process, service, http } = port
  const tls = http?.tls

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-1.5">
        <h2 className="selectable truncate text-xl font-semibold">
          <span className="font-mono">{port.port}</span>
          <span className="ml-2 font-normal text-fg-muted">{ownerLabel(port)}</span>
        </h2>
        <div className="flex items-center gap-2">
          {port.listening ? <Badge tone="success">监听中</Badge> : <Badge>未监听</Badge>}
          {port.bind && (
            <Badge tone={bindLabels[port.bind].tone}>{bindLabels[port.bind].label}</Badge>
          )}
          {port.protocol && <Badge>{port.protocol.toUpperCase()}</Badge>}
        </div>
      </header>

      {port.listening && (
        <Section
          title="监听"
          divided
          actions={
            <Button size="sm" disabled={forwarding} onClick={onForward}>
              <ExternalLink />
              {forwarding ? '转发中…' : '转发到本机并打开'}
            </Button>
          }
        >
          <Row label="地址" value={port.addresses.join('\n')} mono />
          {port.bind && <Row label="可达范围" value={bindLabels[port.bind].text} />}
          {port.pids.length > 1 && <Row label="进程" value={port.pids.join(' ')} mono />}
        </Section>
      )}

      {process && (
        <Section title="进程" divided>
          <Row label="名称" value={`${process.name}（PID ${process.pid}）`} />
          {process.user && <Row label="用户" value={process.user} />}
          {process.started_at && <Row label="启动" value={formatAgo(process.started_at)} />}
          {process.command && <Row label="命令行" value={process.command} mono />}
        </Section>
      )}

      {service && (
        <Section title={service.kind === 'docker' ? 'Docker 容器' : 'systemd 单元'} divided>
          <Row label="名称" value={serviceLabel(service)} />
          {service.image && <Row label="镜像" value={service.image} mono />}
          {service.container_id && <Row label="容器 ID" value={service.container_id} mono />}
          {service.compose_project && (
            <Row
              label="Compose"
              value={[service.compose_project, service.compose_service].filter(Boolean).join(' / ')}
            />
          )}
        </Section>
      )}

      {port.listening && (
        <Section
          title="探测"
          description="从服务器本机通过 SSH 通道发出请求，不需要开放端口。"
          divided
          actions={
            <Button size="sm" disabled={probing} onClick={() => onProbe(serverName)}>
              {probing ? '探测中…' : '重新探测'}
            </Button>
          }
        >
          <FormRow label="域名" description="多个站点共用端口时，填写 Host 头和 SNI 使用的域名。">
            <Input
              aria-label="域名"
              placeholder="example.com"
              value={serverName}
              onChange={(event) => setServerName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !probing) onProbe(serverName)
              }}
            />
          </FormRow>
          {http ? (
            <>
              <FormRow label="响应">
                <span className="flex items-center gap-2 text-sm text-fg-muted">
                  <Badge tone={statusTone(http.status)}>
                    {http.scheme.toUpperCase()} {http.status}
                  </Badge>
                  {http.latency_ms} ms
                </span>
              </FormRow>
              {http.title && <Row label="标题" value={http.title} />}
              {http.server && <Row label="Server" value={http.server} mono />}
              {http.content_type && <Row label="类型" value={http.content_type} mono />}
              {http.location && <Row label="跳转到" value={http.location} mono />}
            </>
          ) : (
            <Row
              label="结果"
              value={
                port.protocol
                  ? `${port.protocol.toUpperCase()}，不是 HTTP 服务`
                  : port.probe_error
                    ? probeErrorLabel(port.probe_error)
                    : '尚未探测'
              }
            />
          )}
          {tls && (
            <>
              <FormRow label="证书">
                <span className="flex items-center gap-2 text-sm text-fg-muted">
                  {tls.days_left !== undefined && (
                    <Badge tone={certTone(tls.days_left)}>{certLabel(tls.days_left)}</Badge>
                  )}
                  {tls.self_signed && <Badge tone="warning">自签名</Badge>}
                  {tls.version}
                </span>
              </FormRow>
              {tls.subject && <Row label="主体" value={tls.subject} mono />}
              {tls.issuer && <Row label="签发者" value={tls.issuer} />}
              {tls.names.length > 0 && <Row label="域名" value={tls.names.join('\n')} mono />}
              {tls.not_after && (
                <Row label="到期" value={new Date(tls.not_after).toLocaleString()} />
              )}
            </>
          )}
        </Section>
      )}

      <Section
        title="备注"
        description="有备注的端口在查找空闲端口时视为已预留。"
        actions={
          port.note && (
            <Button
              size="sm"
              variant="ghost"
              disabled={savingNote}
              onClick={() => onSaveNote({ name: '', project: '', note: '' })}
            >
              清除
            </Button>
          )
        }
      >
        <form
          className="mt-2 flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault()
            onSaveNote(draft)
          }}
        >
          <div className="grid grid-cols-2 gap-3">
            <Field label="名称">
              <Input
                placeholder="例如 api"
                maxLength={100}
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              />
            </Field>
            <Field label="项目">
              <Input
                placeholder="例如 shop"
                maxLength={100}
                value={draft.project}
                onChange={(event) => setDraft({ ...draft, project: event.target.value })}
              />
            </Field>
          </div>
          <Field label="说明">
            <Textarea
              rows={3}
              maxLength={2000}
              value={draft.note}
              onChange={(event) => setDraft({ ...draft, note: event.target.value })}
            />
          </Field>
          <div className="flex items-center justify-end gap-3">
            {port.note && (
              <span className="text-sm text-fg-subtle">
                更新于 {formatAgo(port.note.updated_at)}
              </span>
            )}
            <Button type="submit" variant="primary" disabled={!dirty || savingNote}>
              {savingNote ? '保存中…' : '保存备注'}
            </Button>
          </div>
        </form>
      </Section>
    </div>
  )
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex min-h-row items-start justify-between gap-6 py-2.5">
      <span className="shrink-0 text-base text-fg">{label}</span>
      <span
        className={cn(
          'selectable min-w-0 text-right text-sm break-all whitespace-pre-line text-fg-muted',
          mono && 'font-mono'
        )}
      >
        {value}
      </span>
    </div>
  )
}
