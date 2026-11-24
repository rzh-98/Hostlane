import { DiffLines } from '@/app/DiffView'
import { formatAgo } from '@/app/format'
import { methodLabels } from '@/app/methods'
import { riskLabels } from '@/app/policy'
import { cn } from '@/lib/cn'
import { Badge, Button, FormRow, Section } from '@/ui'
import type { Approval } from '@shared/protocol/Approval'
import type { ApprovalScope } from '@shared/protocol/ApprovalScope'
import { matchedLabel, resolutionLabel, stateBadge } from './describe'

interface ApprovalDetailProps {
  approval: Approval
  now: number
  busy: boolean
  onApprove: (scope: ApprovalScope) => void
  onDeny: () => void
}

const envTones = { dev: 'success', staging: 'warning', prod: 'danger' } as const

export function ApprovalDetail({ approval, now, busy, onApprove, onDeny }: ApprovalDetailProps) {
  const risk = riskLabels[approval.risk]
  const state = stateBadge(approval)
  const pending = approval.state === 'pending'
  const method = methodLabels[approval.method] ?? approval.method
  const client = [approval.client.name, approval.client.session].filter(Boolean).join(' · ')

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <h2 className="truncate text-xl font-semibold">
            {approval.client.name} 请求{method}
          </h2>
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={risk.tone}>{risk.label}</Badge>
            {approval.host && <Badge>{approval.host}</Badge>}
            {approval.env && <Badge tone={envTones[approval.env]}>{approval.env}</Badge>}
            <Badge tone={state.tone}>{state.label}</Badge>
          </div>
        </div>
        {pending && (
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="primary" disabled={busy} onClick={() => onApprove('once')}>
              本次允许
            </Button>
            <Button
              disabled={busy}
              title="同一 agent 会话在这台主机上的相同调用不再询问"
              onClick={() => onApprove('session')}
            >
              本会话允许
            </Button>
            <Button
              disabled={busy}
              title="添加一条允许规则，之后的相同调用都不再询问"
              onClick={() => onApprove('always')}
            >
              总是允许
            </Button>
            <Button variant="ghost" disabled={busy} onClick={onDeny}>
              拒绝
            </Button>
          </div>
        )}
      </header>

      {approval.command !== undefined ? (
        <Section title="命令">
          <pre className="selectable mt-2 overflow-x-auto rounded-md border border-border bg-bg-subtle p-3 font-mono text-sm whitespace-pre-wrap break-all text-fg">
            {approval.cwd && (
              <span className="text-fg-subtle select-none">
                {approval.cwd}
                {'\n'}
              </span>
            )}
            <span className="text-fg-subtle select-none">$ </span>
            {approval.command}
          </pre>
        </Section>
      ) : (
        <Section title="操作">
          <p className="selectable mt-2 font-mono text-sm break-all text-fg">{approval.summary}</p>
        </Section>
      )}

      {approval.diff && (
        <Section title="改动">
          <pre className="selectable mt-2 max-h-[28rem] overflow-auto rounded-md border border-border bg-bg-subtle py-2 font-mono text-sm">
            <DiffLines diff={approval.diff} />
          </pre>
        </Section>
      )}

      <Section title="风险" divided>
        <Row label="等级" value={risk.label} />
        <Row label="原因" value={approval.reasons.length > 0 ? approval.reasons.join('\n') : '—'} />
        <Row label="依据" value={matchedLabel(approval.matched)} />
      </Section>

      <Section title="来源" divided>
        <Row label="调用方" value={client} />
        <Row label="主机" value={approval.host ?? '—'} />
        <Row label="环境" value={approval.env ?? '未标记'} />
        {approval.tags.length > 0 && <Row label="标签" value={approval.tags.join(', ')} />}
        {approval.path && <Row label="路径" value={approval.path} mono />}
        <Row label="方法" value={approval.method} mono />
        <Row label="请求于" value={formatAgo(approval.created_at, now)} />
        {pending && <Row label="过期" value={expiresLabel(approval.expires_at, now)} />}
        <Row label="ID" value={approval.id} mono />
      </Section>

      {!pending && (
        <Section title="结果" divided>
          <Row label="决定" value={state.label} />
          {approval.resolved_by && <Row label="审批人" value={resolutionLabel(approval)} />}
          {approval.resolved_at && (
            <Row label="决定于" value={formatAgo(approval.resolved_at, now)} />
          )}
          {approval.note && <Row label="备注" value={approval.note} />}
        </Section>
      )}
    </div>
  )
}

function expiresLabel(expiresAt: number, now: number): string {
  const minutes = Math.max(0, Math.round((expiresAt - now) / 60_000))
  return minutes >= 1 ? `${minutes} 分钟后` : '即将过期'
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <FormRow label={label}>
      <span
        className={cn(
          'selectable text-sm break-all whitespace-pre-line text-fg-muted',
          mono && 'font-mono'
        )}
      >
        {value}
      </span>
    </FormRow>
  )
}
