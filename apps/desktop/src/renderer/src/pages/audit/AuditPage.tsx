import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { RefreshCw, ScrollText, TriangleAlert } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { DiffDialog } from '@/app/DiffDialog'
import { useHostlaneEvent } from '@/app/events'
import { formatDuration, formatTime } from '@/app/format'
import { methodLabels } from '@/app/methods'
import { approverLabel, riskLabels } from '@/app/policy'
import { OutputDialog } from '@/app/OutputDialog'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  Button,
  EmptyState,
  FormRow,
  IconButton,
  Input,
  List,
  ListItem,
  Notice,
  Section,
  Select
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { AuditEntry } from '@shared/protocol/AuditEntry'
import type { AuditList } from '@shared/protocol/AuditList'
import type { AuditListParams } from '@shared/protocol/AuditListParams'
import type { HostsList } from '@shared/protocol/HostsList'
import type { Risk } from '@shared/protocol/Risk'
import { TaskRollback } from './TaskRollback'

const PAGE_SIZE = 100
const ANY = '*'
const REFRESH_DELAY_MS = 800

const ranges = {
  [ANY]: { label: '全部时间', ms: 0 },
  '1h': { label: '最近 1 小时', ms: 3_600_000 },
  '24h': { label: '最近 24 小时', ms: 86_400_000 },
  '7d': { label: '最近 7 天', ms: 7 * 86_400_000 }
} as const
type Range = keyof typeof ranges

const errorLabels: Partial<Record<string, string>> = {
  HOST_KEY_UNKNOWN: '密钥未信任',
  HOST_KEY_CHANGED: '指纹已变化',
  CONNECTION_LOST: '连接断开',
  AUTH_FAILED: '认证失败',
  HOST_UNREACHABLE: '无法连接',
  HOST_NOT_FOUND: '主机不存在',
  INVALID_PARAMS: '参数错误',
  FILE_CHANGED: '文件已变化',
  FILE_NOT_FOUND: '文件不存在',
  PERMISSION_DENIED: '无权限',
  PATCH_FAILED: '补丁不匹配',
  FILE_ERROR: '文件错误',
  REMOTE_COMMAND_FAILED: '远程命令失败',
  PORT_IN_USE: '端口被占用',
  TUNNEL_NOT_FOUND: '隧道不存在',
  FORWARDING_DENIED: '禁止转发',
  POLICY_DENIED: '策略拒绝',
  APPROVAL_REQUIRED: '等待审批',
  APPROVAL_NOT_FOUND: '审批不存在',
  RULE_NOT_FOUND: '规则不存在'
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : {}
}

function summary(entry: AuditEntry): string {
  const params = record(entry.params)
  if (entry.method === 'approvals.resolve') {
    const resolved = record(entry.result)
    return `${params.approve ? '允许' : '拒绝'} ${String(resolved.summary ?? params.id ?? '')}`
  }
  if (entry.method === 'hosts.label') {
    const tags = Array.isArray(params.tags) ? params.tags.map(String) : []
    return [typeof params.env === 'string' ? params.env : '无环境', ...tags].join(' ')
  }
  if (entry.method === 'policy.stop' && typeof params.reason === 'string') return params.reason
  if (typeof params.command === 'string') return params.command
  if (typeof params.fingerprint === 'string') return params.fingerprint
  if (typeof params.pattern === 'string') return params.pattern
  const result = record(entry.result)
  for (const value of [params.path, params.remote_path, result.path, params.change_id]) {
    if (typeof value === 'string') return value
  }
  if (typeof params.port === 'number') return `:${params.port}`
  const tunnel = record(result.tunnel)
  if (typeof tunnel.url === 'string') {
    return `${tunnel.url} → ${String(tunnel.remote_host)}:${String(tunnel.remote_port)}`
  }
  if (typeof params.remote_port === 'number') return `→ :${params.remote_port}`
  if (params.reason === 'expired') return '到期自动关闭'
  if (typeof params.id === 'string') return params.id
  if (typeof params.session === 'string') return `会话 ${params.session}`
  if (typeof result.port_count === 'number') return `${result.port_count} 个端口`
  if (Array.isArray(result.ports)) return result.ports.join(' ')
  return ''
}

function changeId(entry: AuditEntry): string | null {
  const id = record(entry.result).change_id
  return typeof id === 'string' ? id : null
}

function outputId(entry: AuditEntry): string | null {
  const id = record(entry.result).output_id
  return typeof id === 'string' ? id : null
}

function outcomeBadge(entry: AuditEntry) {
  if (entry.outcome === 'interrupted') return <Badge tone="warning">中断</Badge>
  if (entry.error_kind === 'APPROVAL_REQUIRED') return <Badge tone="accent">等待审批</Badge>
  if (entry.outcome === 'error') {
    return (
      <Badge tone="danger">{(entry.error_kind && errorLabels[entry.error_kind]) ?? '失败'}</Badge>
    )
  }
  const result = record(entry.result)
  if (result.status === 'timed_out') return <Badge tone="warning">超时</Badge>
  if (result.status === 'cancelled') return <Badge tone="warning">急停终止</Badge>
  if (typeof result.exit_code === 'number' && result.exit_code !== 0) {
    return <Badge tone="warning">退出 {result.exit_code}</Badge>
  }
  return <Badge tone="success">成功</Badge>
}

function clientLabel(entry: AuditEntry): string {
  const name = entry.client.verified ? `${entry.client.name} ✓` : entry.client.name
  return entry.client.session ? `${name} · ${entry.client.session}` : name
}

export function AuditPage() {
  const { phase } = daemonStore.use()
  const online = phase === 'online'
  const [host, setHost] = useState(ANY)
  const [client, setClient] = useState(ANY)
  const [groupBy, setGroupBy] = useState('session')
  const [group, setGroup] = useState('')
  const [range, setRange] = useState<Range>(ANY)
  const [minRisk, setMinRisk] = useState<string>(ANY)
  const [entries, setEntries] = useState<AuditEntry[]>([])
  const [hasMore, setHasMore] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<HostlaneError | null>(null)
  const [hosts, setHosts] = useState<string[]>([])
  const [clients, setClients] = useState<string[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [viewing, setViewing] = useState<AuditEntry | null>(null)
  const [diffing, setDiffing] = useState<string | null>(null)
  const request = useRef(0)
  const [exporting, setExporting] = useState(false)
  const [exportFormat, setExportFormat] = useState<'csv' | 'jsonl'>('jsonl')
  const [exported, setExported] = useState('')
  const scope = useCallback(
    (): AuditListParams => ({
      host: host === ANY ? undefined : host,
      client: client === ANY ? undefined : client,
      [groupBy === 'task' ? 'task' : 'session']: group.trim() || undefined,
      since: range === ANY ? undefined : Date.now() - ranges[range].ms,
      min_risk: minRisk === ANY ? undefined : (minRisk as Risk)
    }),
    [host, client, groupBy, group, range, minRisk]
  )
  const save = async () => {
    if (!window.desktop || exporting) return
    setExporting(true)
    setExported('')
    try {
      const result = await window.desktop.hostlane.exportAudit(exportFormat, scope())
      if (!result.ok) setError(result.error)
      else if (result.result) setExported(`已导出到 ${result.result}`)
    } catch {
      setError({ kind: 'IO', message: '导出失败。' })
    } finally {
      setExporting(false)
    }
  }

  const load = useCallback(
    async (before?: number) => {
      const id = ++request.current
      const params: AuditListParams = { ...scope(), limit: PAGE_SIZE, before_id: before }
      setLoading(true)
      const reply = await callHostlane<AuditList>('audit.list', params)
      if (id !== request.current) return
      setLoading(false)
      setLoaded(true)
      if (!reply.ok) {
        setError(reply.error)
        return
      }
      setError(null)
      setHasMore(reply.result.has_more)
      setEntries((previous) =>
        before === undefined ? reply.result.entries : [...previous, ...reply.result.entries]
      )
      setClients((previous) => {
        const names = new Set(previous)
        for (const entry of reply.result.entries) names.add(entry.client.name)
        return names.size === previous.length ? previous : [...names].sort()
      })
    },
    [scope]
  )

  useEffect(() => {
    if (online) void load()
  }, [online, load])

  useEffect(() => {
    if (!online) return
    void callHostlane<HostsList>('hosts.list').then((reply) => {
      if (reply.ok) setHosts(reply.result.hosts.map((h) => h.id))
    })
  }, [online])

  const refreshTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(refreshTimer.current), [])
  useHostlaneEvent((event) => {
    if (event.type !== 'audit_appended' && event.type !== 'subscribed') return
    if (entries.length > PAGE_SIZE) return
    window.clearTimeout(refreshTimer.current)
    refreshTimer.current = window.setTimeout(() => void load(), REFRESH_DELAY_MS)
  })

  const selected = useMemo(
    () => entries.find((e) => String(e.id) === selectedId) ?? null,
    [entries, selectedId]
  )

  const filtered = host !== ANY || client !== ANY || range !== ANY || minRisk !== ANY || !!group

  return (
    <Page>
      <Toolbar
        title="审计"
        subtitle={loaded ? `${entries.length}${hasMore ? '+' : ''} 条` : undefined}
      >
        <Select
          aria-label="主机"
          value={host}
          onValueChange={setHost}
          disabled={!online}
          options={[
            { value: ANY, label: '全部主机' },
            ...[...new Set([...hosts, ...(host === ANY ? [] : [host])])].map((h) => ({
              value: h,
              label: h
            }))
          ]}
        />
        <Select
          aria-label="来源"
          value={client}
          onValueChange={setClient}
          disabled={!online}
          options={[
            { value: ANY, label: '全部来源' },
            ...clients.map((c) => ({ value: c, label: c }))
          ]}
        />
        <Select
          aria-label="风险"
          value={minRisk}
          onValueChange={setMinRisk}
          disabled={!online}
          options={[
            { value: ANY, label: '全部风险' },
            { value: 'L1', label: 'L1 及以上' },
            { value: 'L2', label: 'L2 及以上' },
            { value: 'L3', label: 'L3' }
          ]}
        />
        <Select
          aria-label="时间"
          value={range}
          onValueChange={(value) => setRange(value as Range)}
          disabled={!online}
          options={Object.entries(ranges).map(([value, { label }]) => ({ value, label }))}
        />
        <IconButton
          label="刷新"
          icon={<RefreshCw className={loading ? 'animate-spin' : undefined} />}
          disabled={!online || loading}
          onClick={() => void load()}
        />
      </Toolbar>
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-page py-1">
        <Select
          aria-label="时间线分组"
          value={groupBy}
          onValueChange={setGroupBy}
          options={[
            { value: 'session', label: '会话时间线' },
            { value: 'task', label: '任务时间线' }
          ]}
        />
        <Input
          aria-label="分组 ID"
          placeholder="输入完整 ID"
          value={group}
          onChange={(event) => setGroup(event.target.value)}
        />
        {selected?.client[groupBy === 'task' ? 'task' : 'session'] && (
          <Button
            onClick={() => setGroup(selected.client[groupBy === 'task' ? 'task' : 'session'] ?? '')}
          >
            查看此组
          </Button>
        )}
        {online && groupBy === 'task' && group.trim() && range === ANY && minRisk === ANY && (
          <TaskRollback
            key={JSON.stringify([group.trim(), host, client])}
            task={group.trim()}
            host={host === ANY ? undefined : host}
            client={client === ANY ? undefined : client}
          />
        )}
        <Select
          aria-label="导出格式"
          value={exportFormat}
          onValueChange={(value) => setExportFormat(value === 'csv' ? 'csv' : 'jsonl')}
          options={[
            { value: 'jsonl', label: 'JSONL' },
            { value: 'csv', label: 'CSV' }
          ]}
        />
        <Button disabled={!online || !window.desktop || exporting} onClick={() => void save()}>
          {exporting ? '导出中…' : '导出筛选结果…'}
        </Button>
      </div>
      {exported && (
        <Notice tone="success" onDismiss={() => setExported('')}>
          {exported}
        </Notice>
      )}

      {online && error && (
        <Notice tone="danger" icon={<TriangleAlert />} onDismiss={() => setError(null)}>
          {error.message}
        </Notice>
      )}

      {!online ? (
        <DaemonOffline />
      ) : loaded && entries.length === 0 ? (
        <EmptyState
          icon={<ScrollText />}
          title={filtered ? '没有符合条件的记录' : '还没有记录'}
          description={
            filtered
              ? '调整筛选条件后再试。'
              : 'agent 执行命令、读写文件、连接主机和信任主机密钥时，都会在这里留下不可修改的记录。'
          }
        />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-[28rem] shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            <List aria-label="审计记录" value={selectedId} onValueChange={setSelectedId}>
              {entries.map((entry) => (
                <ListItem
                  key={entry.id}
                  value={String(entry.id)}
                  title={
                    <span className="flex min-w-0 items-baseline gap-2">
                      <span className="shrink-0">{methodLabels[entry.method] ?? entry.method}</span>
                      <span className="truncate font-mono text-sm text-fg-muted">
                        {summary(entry)}
                      </span>
                    </span>
                  }
                  description={[formatTime(entry.started_at), clientLabel(entry), entry.host]
                    .filter(Boolean)
                    .join(' · ')}
                  trailing={
                    <>
                      {entry.risk && entry.risk !== 'L0' && (
                        <Badge tone={riskLabels[entry.risk].tone}>{entry.risk}</Badge>
                      )}
                      {outcomeBadge(entry)}
                    </>
                  }
                />
              ))}
            </List>
            {hasMore && (
              <Button
                className="mt-2 self-center"
                size="sm"
                variant="ghost"
                disabled={loading}
                onClick={() => void load(entries[entries.length - 1]?.id)}
              >
                加载更早的记录
              </Button>
            )}
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected ? (
              <AuditDetail
                entry={selected}
                onViewOutput={() => setViewing(selected)}
                onViewDiff={setDiffing}
              />
            ) : (
              <EmptyState icon={<ScrollText />} title="选择一条记录查看详情" />
            )}
          </div>
        </PageContent>
      )}

      <OutputDialog
        outputId={viewing ? outputId(viewing) : null}
        title={viewing ? summary(viewing) : ''}
        onClose={() => setViewing(null)}
      />
      <DiffDialog changeId={diffing} onClose={() => setDiffing(null)} />
    </Page>
  )
}

interface AuditDetailProps {
  entry: AuditEntry
  onViewOutput: () => void
  onViewDiff: (changeId: string) => void
}

function AuditDetail({ entry, onViewOutput, onViewDiff }: AuditDetailProps) {
  const output = outputId(entry)
  const change = changeId(entry)
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-1.5">
        <div className="flex items-center gap-2">
          <h2 className="truncate text-xl font-semibold">
            {methodLabels[entry.method] ?? entry.method}
          </h2>
          {outcomeBadge(entry)}
        </div>
        <span className="selectable font-mono text-sm text-fg-muted">
          #{entry.id} · {entry.method}
        </span>
      </header>

      <Section
        title="调用"
        divided
        actions={
          output ? (
            <Button size="sm" onClick={onViewOutput}>
              查看完整输出
            </Button>
          ) : (
            change && (
              <Button size="sm" onClick={() => onViewDiff(change)}>
                查看 diff
              </Button>
            )
          )
        }
      >
        <Row label="时间" value={new Date(entry.started_at).toLocaleString()} />
        <Row label="耗时" value={formatDuration(entry.finished_at - entry.started_at)} />
        <Row
          label="来源"
          value={`${clientLabel(entry)}${entry.client.verified ? '（令牌已验证）' : ''}`}
        />
        <Row label="主机" value={entry.host ?? '—'} />
        <Row label="风险" value={entry.risk ? riskLabels[entry.risk].label : '—'} />
        <Row label="审批人" value={entry.approver ? approverLabel(entry.approver) : '无需审批'} />
        {entry.error_kind && <Row label="错误类型" value={entry.error_kind} />}
        {entry.error_message && <Row label="错误信息" value={entry.error_message} />}
      </Section>

      <Section title="参数">
        <Json value={entry.params} />
      </Section>
      <Section title="结果">
        <Json value={entry.result} />
      </Section>
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <FormRow label={label}>
      <span className="selectable text-sm break-all whitespace-pre-line text-fg-muted">
        {value}
      </span>
    </FormRow>
  )
}

function Json({ value }: { value: unknown }) {
  return (
    <pre className="selectable mt-2 overflow-x-auto rounded-md border border-border bg-bg-subtle p-3 font-mono text-sm whitespace-pre-wrap break-all text-fg-muted">
      {value === null || value === undefined ? '—' : JSON.stringify(value, null, 2)}
    </pre>
  )
}
