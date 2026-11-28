import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CircleCheck, Network, RefreshCw, Search, TriangleAlert } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { formatAgo } from '@/app/format'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  Button,
  ConfirmDialog,
  EmptyState,
  IconButton,
  Input,
  List,
  ListItem,
  Notice,
  Select
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { FreePorts } from '@shared/protocol/FreePorts'
import type { HostsList } from '@shared/protocol/HostsList'
import type { ListeningPort } from '@shared/protocol/ListeningPort'
import type { PortAnnotated } from '@shared/protocol/PortAnnotated'
import type { PortsList } from '@shared/protocol/PortsList'
import type { ServiceCommandResult } from '@shared/protocol/ServiceCommandResult'
import type { TunnelOpened } from '@shared/protocol/TunnelOpened'
import type { TunnelsList } from '@shared/protocol/TunnelsList'
import { browserUrl, remoteHostFor } from '@/pages/tunnels/describe'
import { answerLabel, bindLabels, certLabel, certTone, ownerLabel, statusTone } from './describe'
import { FreePortDialog, type FreePortQuery } from './FreePortDialog'
import { LogsDialog, type LogsState } from './LogsDialog'
import { PortDetail, type NoteDraft, type ServiceAction } from './PortDetail'

const HOST_KEY = 'hostlane.ports.host'

type Feedback = { tone: 'success' | 'danger'; text: string; hint?: string }

function errorFeedback(error: HostlaneError): Feedback {
  return { tone: 'danger', text: error.message, hint: error.hint }
}

type PendingAction = { action: ServiceAction | 'kill'; port: ListeningPort }

const actionVerbs: Record<PendingAction['action'], string> = {
  restart: '重启',
  stop: '停止',
  kill: '结束'
}

function firstLine(text: string): string {
  return text.trim().split('\n')[0] ?? ''
}

function matches(port: ListeningPort, query: string): boolean {
  const text = [
    port.port,
    port.process?.name,
    port.process?.command,
    port.service?.name,
    port.service?.compose_project,
    port.http?.title,
    port.http?.server,
    port.protocol,
    port.note?.name,
    port.note?.project,
    port.note?.note
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
  return text.includes(query)
}

export function PortsPage() {
  const { phase } = daemonStore.use()
  const online = phase === 'online'
  const [hosts, setHosts] = useState<string[]>([])
  const [host, setHost] = useState<string | null>(() => localStorage.getItem(HOST_KEY))
  const [data, setData] = useState<PortsList | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<HostlaneError | null>(null)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [probing, setProbing] = useState(false)
  const [logs, setLogs] = useState<LogsState | null>(null)
  const [pending, setPending] = useState<PendingAction | null>(null)
  const [acting, setActing] = useState(false)
  const [savingNote, setSavingNote] = useState(false)
  const [findingFree, setFindingFree] = useState(false)
  const [freeOpen, setFreeOpen] = useState(false)
  const [forwarding, setForwarding] = useState(false)
  const request = useRef(0)

  useEffect(() => {
    if (!online) return
    void callHostlane<HostsList>('hosts.list').then((reply) => {
      if (!reply.ok) return
      const ids = reply.result.hosts.map((h) => h.id)
      setHosts(ids)
      setHost((current) => (current && ids.includes(current) ? current : (ids[0] ?? null)))
    })
  }, [online])

  const load = useCallback(async (target: string) => {
    const id = ++request.current
    setLoading(true)
    const reply = await callHostlane<PortsList>('ports.list', { host: target })
    if (id !== request.current) return
    setLoading(false)
    if (!reply.ok) {
      setLoadError(reply.error)
      return
    }
    setLoadError(null)
    setData(reply.result)
    setSelectedId((current) =>
      reply.result.ports.some((p) => String(p.port) === current)
        ? current
        : reply.result.ports[0]
          ? String(reply.result.ports[0].port)
          : null
    )
  }, [])

  useEffect(() => {
    if (!online || !host) return
    localStorage.setItem(HOST_KEY, host)
    setData((current) => (current?.host === host ? current : null))
    setFeedback(null)
    void load(host)
  }, [online, host, load])

  const replacePort = (port: ListeningPort, remove = false) => {
    setData((current) => {
      if (!current) return current
      const others = current.ports.filter((p) => p.port !== port.port)
      const ports = remove ? others : [...others, port].sort((a, b) => a.port - b.port)
      return { ...current, ports }
    })
  }

  const probe = async (port: number, serverName: string) => {
    if (!host) return
    setProbing(true)
    setFeedback(null)
    const reply = await callHostlane<PortsList>('ports.inspect', {
      host,
      port,
      server_name: serverName.trim() || undefined
    })
    setProbing(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    const fresh = reply.result.ports[0]
    if (fresh) replacePort(fresh, !fresh.listening && !fresh.note)
  }

  const saveNote = async (port: ListeningPort, draft: NoteDraft) => {
    if (!host) return
    setSavingNote(true)
    const reply = await callHostlane<PortAnnotated>('ports.annotate', {
      host,
      port: port.port,
      ...draft
    })
    setSavingNote(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    const note = reply.result.note
    replacePort({ ...port, note }, !port.listening && !note)
    setFeedback({
      tone: 'success',
      text: note ? `已保存端口 ${port.port} 的备注` : `已清除端口 ${port.port} 的备注`
    })
  }

  const forward = async (port: ListeningPort) => {
    if (!host) return
    setForwarding(true)
    setFeedback(null)
    const remoteHost = remoteHostFor(port.addresses)
    const existing = await callHostlane<TunnelsList>('tunnels.list')
    let tunnel = existing.ok
      ? existing.result.tunnels.find(
          (t) =>
            t.host === host &&
            t.remote_port === port.port &&
            t.remote_host === remoteHost &&
            t.state === 'active'
        )
      : undefined
    let warnings: string[] = []
    if (!tunnel) {
      const reply = await callHostlane<TunnelOpened>('tunnels.open', {
        host,
        remote_port: port.port,
        remote_host: remoteHost,
        name: port.note?.name ?? undefined
      })
      if (!reply.ok) {
        setForwarding(false)
        setFeedback(errorFeedback(reply.error))
        return
      }
      tunnel = reply.result.tunnel
      warnings = reply.result.warnings
    }
    setForwarding(false)
    const url = browserUrl(tunnel, port.http?.scheme)
    await window.desktop?.openExternal(url)
    setFeedback({
      tone: 'success',
      text: `已把 ${host}:${port.port} 转发到 ${url}`,
      hint: warnings.length > 0 ? warnings.join('；') : '可以在隧道页查看和保存这条隧道'
    })
  }

  const loadLogs = async (target: Pick<LogsState, 'host' | 'service'>, lines: number) => {
    setLogs((current) => ({
      ...target,
      loading: true,
      result: current?.service === target.service ? current.result : null,
      error: null
    }))
    const reply = await callHostlane<ServiceCommandResult>('services.logs', {
      host: target.host,
      kind: target.service.kind,
      name: target.service.name,
      user_unit: target.service.user_unit,
      lines
    })
    setLogs((current) =>
      current?.service !== target.service
        ? current
        : reply.ok
          ? { ...current, loading: false, result: reply.result }
          : { ...current, loading: false, error: reply.error.message }
    )
  }

  const runAction = async ({ action, port }: PendingAction) => {
    const { service, process } = port
    if (!host || (action === 'kill' ? !process : !service)) return
    setActing(true)
    setFeedback(null)
    const reply =
      action === 'kill'
        ? await callHostlane<ServiceCommandResult>('processes.kill', { host, pid: process?.pid })
        : await callHostlane<ServiceCommandResult>(`services.${action}`, {
            host,
            kind: service?.kind,
            name: service?.name,
            user_unit: service?.user_unit
          })
    setActing(false)
    const target =
      action === 'kill' ? `进程 ${port.process?.name}（PID ${port.process?.pid}）` : service?.name
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    const result = reply.result
    if (result.exit_code === 0) {
      setFeedback({ tone: 'success', text: `已${actionVerbs[action]} ${target}` })
      void load(host)
    } else {
      setFeedback({
        tone: 'danger',
        text: `${actionVerbs[action]} ${target} 失败：${firstLine(result.stderr) || firstLine(result.stdout) || `退出码 ${result.exit_code}`}`,
        hint: result.hint
      })
    }
  }

  const findFree = async (search: FreePortQuery) => {
    if (!host) return
    setFindingFree(true)
    const reply = await callHostlane<FreePorts>('ports.find_free', { host, ...search })
    setFindingFree(false)
    setFreeOpen(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    const { ports, warnings } = reply.result
    setFeedback(
      ports.length > 0
        ? {
            tone: 'success',
            text: `${host} 上的空闲端口：${ports.join('、')}`,
            hint: '有备注的端口视为已预留，不会出现在结果中'
          }
        : { tone: 'danger', text: warnings[0] ?? '该范围内没有空闲端口' }
    )
  }

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase()
    const ports = data?.ports ?? []
    return q ? ports.filter((p) => matches(p, q)) : ports
  }, [data, query])

  const selected = data?.ports.find((p) => String(p.port) === selectedId) ?? null
  const listening = data?.ports.filter((p) => p.listening).length ?? 0

  return (
    <Page>
      <Toolbar
        title="端口"
        subtitle={data && data.host === host ? `${listening} 个监听` : undefined}
      >
        <Select
          aria-label="主机"
          value={host ?? undefined}
          onValueChange={setHost}
          disabled={!online || hosts.length === 0}
          placeholder="选择主机"
          options={hosts.map((h) => ({ value: h, label: h }))}
        />
        <Input
          type="search"
          aria-label="搜索端口"
          placeholder="搜索"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          leading={<Search />}
          wrapperClassName="w-48"
          disabled={!online || !data}
        />
        <Button disabled={!online || !host} onClick={() => setFreeOpen(true)}>
          查找空闲端口
        </Button>
        <IconButton
          label="重新扫描"
          icon={<RefreshCw className={loading ? 'animate-spin' : undefined} />}
          disabled={!online || !host || loading}
          onClick={() => host && void load(host)}
        />
      </Toolbar>

      {online && loadError && (
        <Notice tone="danger" icon={<TriangleAlert />} onDismiss={() => setLoadError(null)}>
          {loadError.message}
          {loadError.hint && <span className="text-fg-muted">。{loadError.hint}</span>}
        </Notice>
      )}
      {online && feedback && (
        <Notice
          tone={feedback.tone}
          icon={feedback.tone === 'success' ? <CircleCheck /> : <TriangleAlert />}
          onDismiss={() => setFeedback(null)}
        >
          <span className="selectable">{feedback.text}</span>
          {feedback.hint && <span className="text-fg-muted">。{feedback.hint}</span>}
        </Notice>
      )}
      {online && data && data.host === host && data.warnings.length > 0 && (
        <Notice tone="warning" icon={<TriangleAlert />}>
          {data.warnings.join('；')}
        </Notice>
      )}

      {!online ? (
        <DaemonOffline />
      ) : hosts.length === 0 ? (
        <EmptyState
          icon={<Network />}
          title="还没有主机"
          description="在主机页或 ~/.ssh/config 中添加主机后，这里会列出它的监听端口。"
        />
      ) : !data || data.host !== host ? (
        <EmptyState
          icon={<Network />}
          title={loading ? `正在扫描 ${host ?? ''}…` : '选择主机查看端口'}
          description={loading ? '列出监听端口，并通过 SSH 探测 HTTP/HTTPS。' : undefined}
        />
      ) : data.ports.length === 0 ? (
        <EmptyState icon={<Network />} title="没有监听中的 TCP 端口" />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-[26rem] shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            {visible.length > 0 ? (
              <List aria-label="端口列表" value={selectedId} onValueChange={setSelectedId}>
                {visible.map((port) => (
                  <PortItem key={port.port} port={port} />
                ))}
              </List>
            ) : (
              <EmptyState icon={<Search />} title="没有匹配的端口" />
            )}
            <p className="mt-auto px-2 pt-3 text-sm text-fg-subtle">
              {data.tool} · 扫描于 {formatAgo(data.scanned_at)}
            </p>
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected ? (
              <PortDetail
                key={`${data.host}:${selected.port}`}
                port={selected}
                probing={probing}
                savingNote={savingNote}
                forwarding={forwarding}
                onForward={() => void forward(selected)}
                onProbe={(serverName) => void probe(selected.port, serverName)}
                onSaveNote={(draft) => void saveNote(selected, draft)}
                acting={acting}
                onLogs={() =>
                  host &&
                  selected.service &&
                  void loadLogs({ host, service: selected.service }, 200)
                }
                onService={(action) => setPending({ action, port: selected })}
                onKill={() => setPending({ action: 'kill', port: selected })}
              />
            ) : (
              <EmptyState icon={<Network />} title="选择端口查看详情" />
            )}
          </div>
        </PageContent>
      )}

      <LogsDialog
        logs={logs}
        onOpenChange={(open) => !open && setLogs(null)}
        onLoad={(lines) => logs && void loadLogs(logs, lines)}
      />
      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(open) => !open && setPending(null)}
        title={
          pending?.action === 'kill'
            ? `结束 ${pending.port.process?.name}（PID ${pending.port.process?.pid}）？`
            : `${pending ? actionVerbs[pending.action] : ''} ${pending?.port.service?.name ?? ''}？`
        }
        description={
          pending?.action === 'kill'
            ? `向进程发送 SIGTERM。如果它由 systemd 或 Docker 管理，可能会被自动拉起；停止服务更可靠。`
            : pending?.action === 'stop'
              ? `${host} 上的 ${pending.port.service?.name} 会停止，端口 ${pending.port.port} 不再提供服务，直到再次启动。`
              : `${host} 上的 ${pending?.port.service?.name} 会短暂中断。系统单元在登录用户不是 root 时使用 sudo -n。`
        }
        confirmLabel={pending ? actionVerbs[pending.action] : '确定'}
        danger={pending?.action !== 'restart'}
        onConfirm={() => {
          if (pending) void runAction(pending)
        }}
      />
      <FreePortDialog
        open={freeOpen}
        host={host ?? ''}
        busy={findingFree}
        onOpenChange={setFreeOpen}
        onSubmit={(search) => void findFree(search)}
      />
    </Page>
  )
}

function PortItem({ port }: { port: ListeningPort }) {
  const days = port.http?.tls?.days_left
  const description = [answerLabel(port), port.note?.project].filter(Boolean).join(' · ')
  return (
    <ListItem
      value={String(port.port)}
      title={
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="shrink-0 font-mono">{port.port}</span>
          <span className="truncate text-fg-muted">{ownerLabel(port)}</span>
        </span>
      }
      description={description || undefined}
      trailing={
        <>
          {days !== undefined && days < 14 && (
            <Badge tone={certTone(days)}>{certLabel(days)}</Badge>
          )}
          {port.http && <Badge tone={statusTone(port.http.status)}>{port.http.status}</Badge>}
          {port.listening ? (
            port.bind && (
              <Badge tone={bindLabels[port.bind].tone}>{bindLabels[port.bind].label}</Badge>
            )
          ) : (
            <Badge>未监听</Badge>
          )}
        </>
      }
    />
  )
}
