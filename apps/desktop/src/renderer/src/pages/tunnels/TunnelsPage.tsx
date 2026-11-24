import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowLeftRight, CircleCheck, TriangleAlert } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { useHostlaneEvent } from '@/app/events'
import { Page, PageContent, Toolbar } from '@/layout'
import { Badge, Button, ConfirmDialog, EmptyState, List, ListItem, Notice } from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { HostsList } from '@shared/protocol/HostsList'
import type { Tunnel } from '@shared/protocol/Tunnel'
import type { TunnelOpened } from '@shared/protocol/TunnelOpened'
import type { TunnelsClosed } from '@shared/protocol/TunnelsClosed'
import type { TunnelsList } from '@shared/protocol/TunnelsList'
import { expiryLabel, isRunning, localLabel, stateLabels, tunnelTitle } from './describe'
import { TunnelDetail } from './TunnelDetail'
import { TunnelDialog, type TunnelDraft } from './TunnelDialog'

type Feedback = { tone: 'success' | 'warning' | 'danger'; text: string; hint?: string }

function errorFeedback(error: HostlaneError): Feedback {
  return { tone: 'danger', text: error.message, hint: error.hint }
}

function openedFeedback(opened: TunnelOpened, verb: string): Feedback {
  const { tunnel, warnings } = opened
  return warnings.length > 0
    ? { tone: 'warning', text: `${verb} ${tunnel.url}`, hint: warnings.join('；') }
    : { tone: 'success', text: `${verb} ${tunnel.url}` }
}

export function TunnelsPage() {
  const { phase } = daemonStore.use()
  const online = phase === 'online'
  const [tunnels, setTunnels] = useState<Tunnel[] | null>(null)
  const [hosts, setHosts] = useState<string[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [busy, setBusy] = useState(false)
  const [dialog, setDialog] = useState<{ tunnel: Tunnel | null } | null>(null)
  const [dialogError, setDialogError] = useState<HostlaneError | null>(null)
  const [deleting, setDeleting] = useState<Tunnel | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const load = useCallback(async () => {
    const reply = await callHostlane<TunnelsList>('tunnels.list')
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    setTunnels(reply.result.tunnels)
  }, [])

  useEffect(() => {
    if (!online) return
    void load()
    void callHostlane<HostsList>('hosts.list').then((reply) => {
      if (reply.ok) setHosts(reply.result.hosts.map((h) => h.id))
    })
  }, [online, load])

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 10_000)
    return () => clearInterval(timer)
  }, [])

  useHostlaneEvent((event) => {
    if (event.type === 'tunnel') {
      const fresh = event.tunnel
      setTunnels((current) => {
        if (!current) return current
        const index = current.findIndex((t) => t.id === fresh.id)
        if (index < 0) return [...current, fresh]
        const next = current.slice()
        next[index] = fresh
        return next
      })
    } else if (event.type === 'tunnel_removed') {
      setTunnels((current) => current?.filter((t) => t.id !== event.id) ?? current)
    } else if (event.type === 'subscribed' || event.type === 'lagged') {
      void load()
    }
  })

  const list = useMemo(() => tunnels ?? [], [tunnels])
  const selected = list.find((t) => t.id === selectedId) ?? list[0] ?? null
  const active = list.filter((t) => t.state === 'active').length

  const replace = (tunnel: Tunnel) =>
    setTunnels((current) =>
      current
        ? current.some((t) => t.id === tunnel.id)
          ? current.map((t) => (t.id === tunnel.id ? tunnel : t))
          : [...current, tunnel]
        : [tunnel]
    )

  const submit = async (draft: TunnelDraft) => {
    const editing = dialog?.tunnel ?? null
    setBusy(true)
    setDialogError(null)
    const reply = editing
      ? await callHostlane<TunnelOpened>('tunnels.update', {
          id: editing.id,
          name: draft.name,
          local_port: draft.localPort ?? undefined,
          remote_host: draft.remoteHost,
          remote_port: draft.remotePort,
          bind: draft.bind,
          allow_public: draft.allowPublic,
          save: draft.save
        })
      : await callHostlane<TunnelOpened>('tunnels.open', {
          host: draft.host,
          remote_port: draft.remotePort,
          remote_host: draft.remoteHost,
          local_port: draft.localPort ?? undefined,
          bind: draft.bind,
          allow_public: draft.allowPublic,
          name: draft.name || undefined,
          save: draft.save
        })
    setBusy(false)
    if (!reply.ok) {
      if (reply.error.kind === 'PORT_IN_USE') setDialogError(reply.error)
      else {
        setDialog(null)
        setFeedback(errorFeedback(reply.error))
      }
      return
    }
    setDialog(null)
    replace(reply.result.tunnel)
    setSelectedId(reply.result.tunnel.id)
    setFeedback(openedFeedback(reply.result, editing ? '已更新' : '已转发到'))
  }

  const toggle = async (tunnel: Tunnel) => {
    setBusy(true)
    setFeedback(null)
    if (isRunning(tunnel)) {
      const reply = await callHostlane<TunnelsClosed>('tunnels.close', { id: tunnel.id })
      setBusy(false)
      if (!reply.ok) setFeedback(errorFeedback(reply.error))
      else if (!tunnel.saved) setFeedback({ tone: 'success', text: `已关闭临时隧道 ${tunnel.url}` })
      return
    }
    const reply = await callHostlane<TunnelOpened>('tunnels.start', { id: tunnel.id })
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    replace(reply.result.tunnel)
    setFeedback(openedFeedback(reply.result, '已转发到'))
  }

  const save = async (tunnel: Tunnel) => {
    setBusy(true)
    const reply = await callHostlane<TunnelOpened>('tunnels.update', { id: tunnel.id, save: true })
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    replace(reply.result.tunnel)
    setFeedback({ tone: 'success', text: `已保存隧道，守护进程启动时会自动建立` })
  }

  const remove = async (tunnel: Tunnel) => {
    setBusy(true)
    const reply = await callHostlane<TunnelsClosed>('tunnels.delete', { id: tunnel.id })
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    setTunnels((current) => current?.filter((t) => t.id !== tunnel.id) ?? current)
    setFeedback({ tone: 'success', text: `已删除隧道 ${tunnelTitle(tunnel)}` })
  }

  return (
    <Page>
      <Toolbar
        title="隧道"
        subtitle={tunnels && tunnels.length > 0 ? `${active} 条运行中` : undefined}
      >
        <Button
          variant="primary"
          disabled={!online || hosts.length === 0}
          onClick={() => {
            setDialogError(null)
            setDialog({ tunnel: null })
          }}
        >
          新建隧道
        </Button>
      </Toolbar>

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

      {!online ? (
        <DaemonOffline />
      ) : !tunnels ? (
        <EmptyState icon={<ArrowLeftRight />} title="正在加载隧道…" />
      ) : tunnels.length === 0 ? (
        <EmptyState
          icon={<ArrowLeftRight />}
          title="还没有隧道"
          description="新建隧道，或在端口页选择 web 服务后“转发到本机”。agent 也可以用 hostlane tunnel open 创建临时隧道。"
        />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-[24rem] shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            <List aria-label="隧道列表" value={selected?.id ?? null} onValueChange={setSelectedId}>
              {list.map((tunnel) => (
                <TunnelItem key={tunnel.id} tunnel={tunnel} now={now} />
              ))}
            </List>
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected && (
              <TunnelDetail
                key={selected.id}
                tunnel={selected}
                busy={busy}
                now={now}
                onToggle={() => void toggle(selected)}
                onEdit={() => {
                  setDialogError(null)
                  setDialog({ tunnel: selected })
                }}
                onSave={() => void save(selected)}
                onDelete={() => setDeleting(selected)}
              />
            )}
          </div>
        </PageContent>
      )}

      <TunnelDialog
        open={dialog !== null}
        tunnel={dialog?.tunnel ?? null}
        hosts={hosts}
        defaultHost={selected?.host ?? null}
        busy={busy}
        error={dialogError}
        onOpenChange={(open) => !open && setDialog(null)}
        onSubmit={(draft) => void submit(draft)}
      />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={`删除隧道 ${deleting ? tunnelTitle(deleting) : ''}？`}
        description="隧道会立即关闭，保存的配置也会删除。"
        confirmLabel="删除"
        danger
        onConfirm={() => deleting && void remove(deleting)}
      />
    </Page>
  )
}

function TunnelItem({ tunnel, now }: { tunnel: Tunnel; now: number }) {
  const state = stateLabels[tunnel.state]
  const description = [
    `${localLabel(tunnel)} → ${tunnel.host}:${tunnel.remote_port}`,
    !tunnel.saved && tunnel.expires_at ? expiryLabel(tunnel.expires_at, now) : null
  ]
    .filter(Boolean)
    .join(' · ')
  return (
    <ListItem
      value={tunnel.id}
      title={
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="truncate">{tunnelTitle(tunnel)}</span>
          {tunnel.created_by.name !== 'desktop' && (
            <span className="shrink-0 text-sm text-fg-muted">{tunnel.created_by.name}</span>
          )}
        </span>
      }
      description={description}
      trailing={
        <>
          {tunnel.active_connections > 0 && <Badge>{tunnel.active_connections} 连接</Badge>}
          <Badge tone={state.tone}>{state.label}</Badge>
        </>
      }
    />
  )
}
