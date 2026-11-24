import { useCallback, useEffect, useMemo, useState } from 'react'
import { RefreshCw, Search, Server, TriangleAlert } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  EmptyState,
  FormRow,
  IconButton,
  Input,
  List,
  ListItem,
  Notice,
  Section
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { Host } from '@shared/protocol/Host'
import type { HostEnv } from '@shared/protocol/HostEnv'
import type { HostsList } from '@shared/protocol/HostsList'

const envBadge: Record<HostEnv, { label: string; tone: 'success' | 'warning' | 'danger' }> = {
  dev: { label: 'dev', tone: 'success' },
  staging: { label: 'staging', tone: 'warning' },
  prod: { label: 'prod', tone: 'danger' }
}

function address(host: Host): string {
  const user = host.user ? `${host.user}@` : ''
  const port = host.port === 22 ? '' : `:${host.port}`
  return `${user}${host.hostname}${port}`
}

export function HostsPage() {
  const { phase } = daemonStore.use()
  const [data, setData] = useState<HostsList | null>(null)
  const [error, setError] = useState<HostlaneError | null>(null)
  const [loading, setLoading] = useState(false)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [query, setQuery] = useState('')

  const load = useCallback(async () => {
    setLoading(true)
    const reply = await callHostlane<HostsList>('hosts.list')
    setLoading(false)
    if (reply.ok) {
      setData(reply.result)
      setError(null)
      setSelectedId((id) =>
        reply.result.hosts.some((h) => h.id === id) ? id : (reply.result.hosts[0]?.id ?? null)
      )
    } else {
      setError(reply.error)
    }
  }, [])

  useEffect(() => {
    if (phase === 'online') void load()
  }, [phase, load])

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase()
    const hosts = data?.hosts ?? []
    if (!q) return hosts
    return hosts.filter((h) =>
      [h.alias, h.hostname, h.user ?? '', ...h.tags].join(' ').toLowerCase().includes(q)
    )
  }, [data, query])

  const selected = data?.hosts.find((h) => h.id === selectedId) ?? null
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

      {online && error && (
        <Notice tone="danger" icon={<TriangleAlert />} onDismiss={() => setError(null)}>
          {error.message}
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
              <List aria-label="主机列表" value={selectedId} onValueChange={setSelectedId}>
                {visible.map((host) => (
                  <ListItem
                    key={host.id}
                    value={host.id}
                    leading={<Server />}
                    title={host.alias}
                    description={address(host)}
                    trailing={
                      host.env ? (
                        <Badge tone={envBadge[host.env].tone}>{envBadge[host.env].label}</Badge>
                      ) : undefined
                    }
                  />
                ))}
              </List>
            ) : (
              data && <EmptyState icon={<Search />} title="没有匹配的主机" />
            )}
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected ? (
              <HostDetail host={selected} />
            ) : (
              <EmptyState icon={<Server />} title="未选择主机" />
            )}
          </div>
        </PageContent>
      )}
    </Page>
  )
}

function HostDetail({ host }: { host: Host }) {
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-page">
      <header className="flex flex-col gap-1.5">
        <h2 className="selectable truncate text-xl font-semibold">{host.alias}</h2>
        <div className="flex items-center gap-2 text-sm text-fg-muted">
          <Badge>{host.source === 'ssh_config' ? '来自 ~/.ssh/config' : '手动添加'}</Badge>
          {host.env && <Badge tone={envBadge[host.env].tone}>{envBadge[host.env].label}</Badge>}
        </div>
      </header>

      <Section title="连接" divided>
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

      <Section title="状态" description="连接状态、延迟和主机画像将在 M1 提供。" />
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <FormRow label={label}>
      <span className="selectable font-mono text-sm whitespace-pre-line text-fg-muted">
        {value}
      </span>
    </FormRow>
  )
}
