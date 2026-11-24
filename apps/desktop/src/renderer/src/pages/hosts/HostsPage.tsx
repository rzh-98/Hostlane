import { useCallback, useEffect, useMemo, useState } from 'react'
import { CircleCheck, RefreshCw, Search, Server, ShieldAlert, TriangleAlert } from 'lucide-react'
import {
  connectionsStore,
  describeConnection,
  loadConnections,
  setConnection
} from '@/app/connections'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { formatAgo } from '@/app/format'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  Button,
  ConfirmDialog,
  EmptyState,
  FormRow,
  IconButton,
  Input,
  List,
  ListItem,
  Notice,
  Section,
  StatusDot
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { Connection } from '@shared/protocol/Connection'
import type { Host } from '@shared/protocol/Host'
import type { HostEnv } from '@shared/protocol/HostEnv'
import type { HostKeyInfo } from '@shared/protocol/HostKeyInfo'
import type { HostsList } from '@shared/protocol/HostsList'
import type { RpcError } from '@shared/protocol/RpcError'
import type { TrustResult } from '@shared/protocol/TrustResult'

const envBadge: Record<HostEnv, { label: string; tone: 'success' | 'warning' | 'danger' }> = {
  dev: { label: 'dev', tone: 'success' },
  staging: { label: 'staging', tone: 'warning' },
  prod: { label: 'prod', tone: 'danger' }
}

type Feedback = { tone: 'success' | 'danger'; text: string; hint?: string }

function address(host: Host): string {
  const user = host.user ? `${host.user}@` : ''
  const port = host.port === 22 ? '' : `:${host.port}`
  return `${user}${host.hostname}${port}`
}

function hostKeyFrom(error: { kind: string; details?: Record<string, unknown> }) {
  return error.kind === 'HOST_KEY_UNKNOWN' && error.details
    ? (error.details as unknown as HostKeyInfo)
    : null
}

/** Plain-language explanation of a failed connection. */
function problemText(host: string, error: RpcError): { text: string; hint?: string } {
  if (error.data.kind === 'HOST_KEY_CHANGED') {
    const recordedAt = (error.data.details as Partial<HostKeyInfo> | undefined)?.recorded_at
    return {
      text: `${host} 的主机密钥与记录不一致，可能有人在拦截连接，Hostlane 已拒绝连接。`,
      hint: recordedAt
        ? `如果确认服务器更换过密钥，删除 ${recordedAt} 中的旧记录后再连接`
        : undefined
    }
  }
  return { text: error.message, hint: error.data.hint }
}

export function HostsPage() {
  const { phase } = daemonStore.use()
  const { byHost } = connectionsStore.use()
  const [data, setData] = useState<HostsList | null>(null)
  const [loadError, setLoadError] = useState<HostlaneError | null>(null)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [loading, setLoading] = useState(false)
  const [busyHost, setBusyHost] = useState<string | null>(null)
  const [pendingTrust, setPendingTrust] = useState<HostKeyInfo | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [query, setQuery] = useState('')

  const load = useCallback(async () => {
    setLoading(true)
    const [reply] = await Promise.all([callHostlane<HostsList>('hosts.list'), loadConnections()])
    setLoading(false)
    if (reply.ok) {
      setData(reply.result)
      setLoadError(null)
      setSelectedId((id) =>
        reply.result.hosts.some((h) => h.id === id) ? id : (reply.result.hosts[0]?.id ?? null)
      )
    } else {
      setLoadError(reply.error)
    }
  }, [])

  useEffect(() => {
    if (phase === 'online') void load()
  }, [phase, load])

  const connect = async (host: string) => {
    setBusyHost(host)
    setFeedback(null)
    const reply = await callHostlane<Connection>('hosts.connect', { host })
    setBusyHost(null)
    if (reply.ok) {
      setConnection(reply.result)
      return
    }
    const hostKey = hostKeyFrom(reply.error)
    if (hostKey) setPendingTrust(hostKey)
    else setFeedback({ tone: 'danger', text: reply.error.message, hint: reply.error.hint })
  }

  const disconnect = async (host: string) => {
    setBusyHost(host)
    setFeedback(null)
    const reply = await callHostlane('hosts.disconnect', { host })
    setBusyHost(null)
    if (!reply.ok) setFeedback({ tone: 'danger', text: reply.error.message })
    void loadConnections()
  }

  const trust = async (key: HostKeyInfo) => {
    setBusyHost(key.host)
    const reply = await callHostlane<TrustResult>('hosts.trust', {
      host: key.host,
      fingerprint: key.fingerprint
    })
    setBusyHost(null)
    void loadConnections()
    setFeedback(
      reply.ok
        ? {
            tone: 'success',
            text: `已信任 ${key.host} 的主机密钥，并写入 ${reply.result.known_hosts_path}`
          }
        : { tone: 'danger', text: reply.error.message, hint: reply.error.hint }
    )
  }

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase()
    const hosts = data?.hosts ?? []
    if (!q) return hosts
    return hosts.filter((h) =>
      [h.alias, h.hostname, h.user ?? '', ...h.tags].join(' ').toLowerCase().includes(q)
    )
  }, [data, query])

  const selected = data?.hosts.find((h) => h.id === selectedId) ?? null
  const selectedConnection = selected ? byHost[selected.id] : undefined
  const problem = selectedConnection?.state === 'failed' ? selectedConnection.error : undefined
  const unknownKey = problem ? hostKeyFrom(problem.data) : null
  const problemInfo = problem && selected ? problemText(selected.id, problem) : null
  const online = phase === 'online'

  return (
    <Page>
      <Toolbar title="主机" subtitle={data ? `${data.hosts.length} 台` : undefined}>
        <Input
          type="search"
          aria-label="搜索主机"
          placeholder="搜索"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          leading={<Search />}
          wrapperClassName="w-56"
          disabled={!online}
        />
        <IconButton
          label="刷新"
          icon={<RefreshCw className={loading ? 'animate-spin' : undefined} />}
          disabled={!online || loading}
          onClick={() => void load()}
        />
      </Toolbar>

      {online && loadError && (
        <Notice tone="danger" icon={<TriangleAlert />} onDismiss={() => setLoadError(null)}>
          {loadError.message}
        </Notice>
      )}
      {online && feedback && (
        <Notice
          tone={feedback.tone}
          icon={feedback.tone === 'success' ? <CircleCheck /> : <TriangleAlert />}
          onDismiss={() => setFeedback(null)}
        >
          {feedback.text}
          {feedback.hint && <span className="text-fg-muted">。{feedback.hint}</span>}
        </Notice>
      )}
      {online && !feedback && problem && (
        <Notice
          tone={unknownKey ? 'warning' : 'danger'}
          icon={<ShieldAlert />}
          actions={
            unknownKey && (
              <Button size="sm" onClick={() => setPendingTrust(unknownKey)}>
                核对指纹…
              </Button>
            )
          }
        >
          {unknownKey
            ? `${unknownKey.host} 的主机密钥尚未信任，agent 无法连接。`
            : problemInfo?.text}
          {!unknownKey && problemInfo?.hint && (
            <span className="text-fg-muted"> {problemInfo.hint}</span>
          )}
        </Notice>
      )}
      {online && data && data.warnings.length > 0 && (
        <Notice tone="warning" icon={<TriangleAlert />}>
          {data.warnings.join('；')}
        </Notice>
      )}

      {!online ? (
        <DaemonOffline />
      ) : data && data.hosts.length === 0 ? (
        <EmptyState
          icon={<Server />}
          title="还没有主机"
          description="Hostlane 会读取 ~/.ssh/config 中的 Host 条目。添加后点击刷新。"
        />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-80 shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            {visible.length > 0 ? (
              <List
                aria-label="主机列表"
                value={selectedId}
                onValueChange={setSelectedId}
                onAction={(id) => {
                  if (byHost[id]?.state !== 'connected') void connect(id)
                }}
              >
                {visible.map((host) => {
                  const status = describeConnection(byHost[host.id])
                  return (
                    <ListItem
                      key={host.id}
                      value={host.id}
                      leading={<Server />}
                      title={host.alias}
                      description={address(host)}
                      trailing={
                        <>
                          {host.env && (
                            <Badge tone={envBadge[host.env].tone}>{envBadge[host.env].label}</Badge>
                          )}
                          <span title={status.label} className="flex">
                            <StatusDot tone={status.tone} />
                          </span>
                        </>
                      }
                    />
                  )
                })}
              </List>
            ) : (
              data && <EmptyState icon={<Search />} title="没有匹配的主机" />
            )}
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected ? (
              <HostDetail
                host={selected}
                connection={selectedConnection}
                busy={busyHost === selected.id}
                onConnect={() => void connect(selected.id)}
                onDisconnect={() => void disconnect(selected.id)}
              />
            ) : (
              <EmptyState icon={<Server />} title="未选择主机" />
            )}
          </div>
        </PageContent>
      )}

      <ConfirmDialog
        open={pendingTrust !== null}
        onOpenChange={(open) => !open && setPendingTrust(null)}
        title={`信任 ${pendingTrust?.host ?? ''} 的主机密钥？`}
        description={
          pendingTrust && (
            <span className="flex flex-col gap-3">
              <span>
                请先通过可信渠道（例如云控制台或服务器管理员）核对指纹，确认一致后再信任。
              </span>
              <span className="selectable flex flex-col gap-0.5 rounded-md border border-border bg-bg-subtle p-2 font-mono text-sm break-all text-fg">
                <span>{pendingTrust.fingerprint}</span>
                <span className="text-fg-muted">
                  {pendingTrust.key_type} · {pendingTrust.known_hosts_name}
                </span>
              </span>
              <span>密钥会写入 Hostlane 自己的 known_hosts，不会修改 ~/.ssh/known_hosts。</span>
            </span>
          )
        }
        confirmLabel="信任并连接"
        onConfirm={() => {
          if (pendingTrust) void trust(pendingTrust)
        }}
      />
    </Page>
  )
}

interface HostDetailProps {
  host: Host
  connection: Connection | undefined
  busy: boolean
  onConnect: () => void
  onDisconnect: () => void
}

function HostDetail({ host, connection, busy, onConnect, onDisconnect }: HostDetailProps) {
  const status = describeConnection(connection)
  const connected = connection?.state === 'connected'
  const hostKey = connection?.host_key

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-1.5">
        <h2 className="selectable truncate text-xl font-semibold">{host.alias}</h2>
        <div className="flex items-center gap-2 text-sm text-fg-muted">
          <Badge>{host.source === 'ssh_config' ? '来自 ~/.ssh/config' : '手动添加'}</Badge>
          {host.env && <Badge tone={envBadge[host.env].tone}>{envBadge[host.env].label}</Badge>}
        </div>
      </header>

      <Section
        title="状态"
        divided
        actions={
          connected ? (
            <Button size="sm" disabled={busy} onClick={onDisconnect}>
              断开
            </Button>
          ) : (
            <Button
              size="sm"
              disabled={busy || connection?.state === 'connecting'}
              onClick={onConnect}
            >
              {busy ? '连接中…' : '连接'}
            </Button>
          )
        }
      >
        <FormRow label="连接">
          <span className="flex items-center gap-2 text-base text-fg">
            <StatusDot tone={status.tone} />
            {status.label}
          </span>
        </FormRow>
        {connected && (
          <>
            <Row
              label="延迟"
              value={
                connection.latency_ms === undefined
                  ? '—'
                  : connection.latency_ms === 0
                    ? '<1 ms'
                    : `${connection.latency_ms} ms`
              }
            />
            <Row label="活动通道" value={String(connection.active_channels)} />
            {connection.connected_at && (
              <Row label="连接时间" value={formatAgo(connection.connected_at)} />
            )}
          </>
        )}
        {connection?.last_used_at && (
          <Row label="最近使用" value={formatAgo(connection.last_used_at)} />
        )}
        {hostKey && (
          <Row
            label="主机指纹"
            value={`${hostKey.fingerprint}\n${hostKey.key_type} · ${hostKey.known_hosts_name}`}
          />
        )}
      </Section>

      <Section title="配置" divided>
        <Row label="主机地址" value={host.hostname} />
        <Row label="用户" value={host.user ?? '默认（当前用户）'} />
        <Row label="端口" value={String(host.port)} />
        <Row label="跳板机" value={host.proxy_jump ?? '无'} />
        <Row
          label="密钥文件"
          value={
            host.identity_files.length > 0 ? host.identity_files.join('\n') : '默认 / ssh-agent'
          }
        />
      </Section>
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <FormRow label={label}>
      <span className="selectable font-mono text-sm break-all whitespace-pre-line text-fg-muted">
        {value}
      </span>
    </FormRow>
  )
}
