import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { ArrowDown, RefreshCw, ShieldCheck, SquareTerminal, TriangleAlert } from 'lucide-react'
import { daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { useHostlaneEvent } from '@/app/events'
import { formatAgo, formatDuration, formatTime } from '@/app/format'
import { OutputDialog } from '@/app/OutputDialog'
import { approverLabel } from '@/app/policy'
import { createStore } from '@/lib/store'
import { cn } from '@/lib/cn'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  Button,
  EmptyState,
  IconButton,
  List,
  ListHeader,
  ListItem,
  Notice,
  SegmentedControl,
  StatusDot
} from '@/ui'
import type { ExecRecord } from '@shared/protocol/ExecRecord'
import { handleJobEvent, jobsStore, loadJobs, resetJobs } from './jobs'
import { JobsView } from './JobsView'
import {
  ensureOutput,
  handleSessionEvent,
  loadSessions,
  resetSessions,
  sessionsStore,
  type ExecOutput
} from './store'

type GroupBy = 'host' | 'session'
type View = 'commands' | 'jobs'

const ALL = '*'

const prefs = createStore<{ groupBy: GroupBy; view?: View }>('sessions.prefs', {
  groupBy: 'host'
})

function sessionKey(exec: ExecRecord): string {
  return exec.client.session ? `${exec.client.name} · ${exec.client.session}` : exec.client.name
}

function groupKey(exec: ExecRecord, groupBy: GroupBy): string {
  return groupBy === 'host' ? exec.host : sessionKey(exec)
}

interface Group {
  key: string
  execs: ExecRecord[]
  running: number
  lastAt: number
}

export function SessionsPage() {
  const { phase } = daemonStore.use()
  const { execs, outputs, loaded, error } = sessionsStore.use()
  const { groupBy, view = 'commands' } = prefs.use()
  const jobs = jobsStore.use()
  const [selected, setSelected] = useState(ALL)
  const [viewing, setViewing] = useState<ExecRecord | null>(null)
  const online = phase === 'online'

  useHostlaneEvent(handleSessionEvent)
  useHostlaneEvent(handleJobEvent)
  useEffect(() => {
    if (online) {
      void loadSessions()
      void loadJobs()
    }
  }, [online])
  useEffect(
    () => () => {
      resetSessions()
      resetJobs()
    },
    []
  )

  const ordered = useMemo(
    () => Object.values(execs).sort((a, b) => a.started_at - b.started_at),
    [execs]
  )

  const groups = useMemo(() => {
    const map = new Map<string, Group>()
    for (const exec of ordered) {
      const key = groupKey(exec, groupBy)
      const group = map.get(key) ?? { key, execs: [], running: 0, lastAt: 0 }
      group.execs.push(exec)
      if (exec.status === 'running') group.running++
      group.lastAt = Math.max(group.lastAt, exec.started_at)
      map.set(key, group)
    }
    return [...map.values()].sort((a, b) => b.lastAt - a.lastAt)
  }, [ordered, groupBy])

  const current = selected === ALL || !groups.some((g) => g.key === selected) ? ALL : selected
  const shown = current === ALL ? ordered : ordered.filter((e) => groupKey(e, groupBy) === current)
  const running = ordered.filter((e) => e.status === 'running').length
  const jobList = Object.values(jobs.jobs)
  const runningJobs = jobList.filter((job) => job.state === 'running').length
  const subtitle =
    view === 'jobs'
      ? runningJobs > 0
        ? `${runningJobs} 个任务运行中`
        : jobs.loaded
          ? `${jobList.length} 个任务`
          : undefined
      : running > 0
        ? `${running} 条命令运行中`
        : loaded
          ? `${ordered.length} 条命令`
          : undefined
  const pageError = view === 'jobs' ? jobs.error : error

  return (
    <Page>
      <Toolbar title="会话" subtitle={subtitle}>
        <SegmentedControl<View>
          aria-label="内容"
          value={view}
          onValueChange={(value) => prefs.set({ view: value })}
          options={[
            { value: 'commands', label: '命令' },
            { value: 'jobs', label: runningJobs > 0 ? `后台任务 ${runningJobs}` : '后台任务' }
          ]}
        />
        {view === 'commands' && (
          <SegmentedControl
            aria-label="分组方式"
            value={groupBy}
            onValueChange={(value) => {
              prefs.set({ groupBy: value })
              setSelected(ALL)
            }}
            options={[
              { value: 'host', label: '按主机' },
              { value: 'session', label: '按会话' }
            ]}
          />
        )}
        <IconButton
          label="刷新"
          icon={<RefreshCw />}
          disabled={!online}
          onClick={() => void (view === 'jobs' ? loadJobs() : loadSessions())}
        />
      </Toolbar>

      {online && pageError && (
        <Notice tone="danger" icon={<TriangleAlert />}>
          {pageError.message}
        </Notice>
      )}

      {!online ? (
        <DaemonOffline />
      ) : view === 'jobs' ? (
        <PageContent padded={false}>
          <JobsView />
        </PageContent>
      ) : loaded && ordered.length === 0 ? (
        <EmptyState
          icon={<SquareTerminal />}
          title="还没有命令"
          description="agent 通过 hostlane exec 执行的命令会实时显示在这里。这里只能查看，不能输入。"
        />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-64 shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            <List
              aria-label={groupBy === 'host' ? '主机' : '会话'}
              value={current}
              onValueChange={setSelected}
            >
              <ListItem
                value={ALL}
                title="全部"
                trailing={<GroupTrailing count={ordered.length} running={running} />}
              />
              <ListHeader>{groupBy === 'host' ? '主机' : 'agent 会话'}</ListHeader>
              {groups.map((group) => (
                <ListItem
                  key={group.key}
                  value={group.key}
                  title={group.key}
                  description={formatAgo(group.lastAt)}
                  trailing={<GroupTrailing count={group.execs.length} running={group.running} />}
                />
              ))}
            </List>
          </div>
          <Transcript
            execs={shown}
            outputs={outputs}
            showHost={groupBy === 'session' || current === ALL}
            showClient={groupBy === 'host' || current === ALL}
            onViewOutput={setViewing}
          />
        </PageContent>
      )}

      <OutputDialog
        outputId={viewing?.id ?? null}
        title={viewing?.command ?? ''}
        onClose={() => setViewing(null)}
      />
    </Page>
  )
}

function GroupTrailing({ count, running }: { count: number; running: number }) {
  return (
    <>
      {running > 0 && (
        <span title={`${running} 条运行中`} className="flex">
          <StatusDot tone="accent" />
        </span>
      )}
      <span className="tabular-nums">{count}</span>
    </>
  )
}

interface TranscriptProps {
  execs: ExecRecord[]
  outputs: Record<string, ExecOutput>
  showHost: boolean
  showClient: boolean
  onViewOutput: (exec: ExecRecord) => void
}

/** Read-only, terminal-like log of commands; follows new output while scrolled to the bottom. */
function Transcript({ execs, outputs, showHost, showClient, onViewOutput }: TranscriptProps) {
  const scroller = useRef<HTMLDivElement>(null)
  const [following, setFollowing] = useState(true)
  const [replaying, setReplaying] = useState(false)
  const [position, setPosition] = useState(0)
  const [playing, setPlaying] = useState(false)
  const index = Math.min(position, Math.max(execs.length - 1, 0))
  useEffect(() => {
    if (!replaying || !playing) return
    if (index + 1 >= execs.length) {
      setPlaying(false)
      return
    }
    const timer = window.setTimeout(() => setPosition(index + 1), 2000)
    return () => window.clearTimeout(timer)
  }, [index, playing, replaying, execs.length])

  useLayoutEffect(() => {
    const node = scroller.current
    if (node && following) node.scrollTop = node.scrollHeight
  })

  return (
    <div className="relative flex min-w-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-page py-1">
        <Button
          disabled={!execs.length}
          onClick={() => {
            setReplaying(!replaying)
            setPosition(0)
            setPlaying(false)
          }}
        >
          {replaying ? '返回实时视图' : '按命令回放'}
        </Button>
        {replaying && (
          <>
            <select
              aria-label="跳转到命令"
              value={index}
              onChange={(event) => {
                setPosition(Number(event.target.value))
                setPlaying(false)
              }}
              className="min-w-0 max-w-64 bg-control"
            >
              {execs.map((exec, number) => (
                <option key={exec.id} value={number}>
                  {number + 1}. {exec.command}
                </option>
              ))}
            </select>
            <Button disabled={index + 1 >= execs.length} onClick={() => setPlaying(!playing)}>
              {playing ? '暂停' : '播放'}
            </Button>
          </>
        )}
      </div>
      <div
        ref={scroller}
        onScroll={(event) => {
          const node = event.currentTarget
          setFollowing(node.scrollHeight - node.scrollTop - node.clientHeight < 24)
        }}
        className="h-full overflow-y-auto"
        aria-live="off"
      >
        <div className="flex flex-col divide-y divide-border">
          {(replaying ? execs.slice(index, index + 1) : execs).map((exec) => (
            <ExecBlock
              key={exec.id}
              exec={exec}
              output={outputs[exec.id]}
              showHost={showHost}
              showClient={showClient}
              onViewOutput={() => onViewOutput(exec)}
            />
          ))}
        </div>
      </div>
      {!following && (
        <Button
          size="sm"
          className="absolute right-4 bottom-4 shadow-popover"
          onClick={() => {
            const node = scroller.current
            if (node) node.scrollTop = node.scrollHeight
            setFollowing(true)
          }}
        >
          <ArrowDown />
          跟随最新
        </Button>
      )}
    </div>
  )
}

function statusBadge(exec: ExecRecord) {
  switch (exec.status) {
    case 'running':
      return <Badge tone="accent">运行中</Badge>
    case 'exited':
      if (exec.signal) return <Badge tone="danger">信号 {exec.signal}</Badge>
      return <Badge tone={exec.exit_code === 0 ? 'success' : 'danger'}>退出 {exec.exit_code}</Badge>
    case 'timed_out':
      return <Badge tone="warning">超时</Badge>
    case 'failed':
      return <Badge tone="danger">连接断开</Badge>
    case 'interrupted':
      return <Badge tone="warning">已中断</Badge>
    case 'cancelled':
      return <Badge tone="warning">急停终止</Badge>
  }
}

interface ExecBlockProps {
  exec: ExecRecord
  output: ExecOutput | undefined
  showHost: boolean
  showClient: boolean
  onViewOutput: () => void
}

function ExecBlock({ exec, output, showHost, showClient, onViewOutput }: ExecBlockProps) {
  useEffect(() => ensureOutput(exec), [exec])
  const hasStored = exec.stdout_bytes + exec.stderr_bytes > 0

  return (
    <section className="flex flex-col gap-1.5 px-page py-3">
      <header className="flex items-center gap-2 text-sm text-fg-muted">
        <span className="font-mono tabular-nums">{formatTime(exec.started_at)}</span>
        {showHost && <Badge>{exec.host}</Badge>}
        {showClient && <span className="truncate">{sessionKey(exec)}</span>}
        {exec.cwd && <span className="truncate font-mono">{exec.cwd}</span>}
        <span className="flex-1" />
        {exec.approver && (
          <span className="flex shrink-0 items-center gap-1 text-success-text" title="审批放行">
            <ShieldCheck className="size-3.5" />
            {approverLabel(exec.approver)}
          </span>
        )}
        {exec.finished_at && (
          <span className="tabular-nums">{formatDuration(exec.finished_at - exec.started_at)}</span>
        )}
        {statusBadge(exec)}
      </header>
      <pre className="selectable font-mono text-sm font-medium whitespace-pre-wrap break-all text-fg">
        <span className="text-fg-subtle select-none">$ </span>
        {exec.command}
      </pre>
      {output?.clipped && <div className="font-mono text-sm text-fg-subtle">…前面的输出已省略</div>}
      {output && output.pieces.length > 0 && (
        <pre className="selectable font-mono text-sm whitespace-pre-wrap break-all text-fg-muted">
          {output.pieces.map((piece, index) => (
            <span key={index} className={cn(piece.stream === 'stderr' && 'text-danger-text')}>
              {piece.data}
            </span>
          ))}
        </pre>
      )}
      {output?.state === 'loading' && <div className="text-sm text-fg-subtle">读取输出…</div>}
      {output?.state === 'error' && (
        <div className="text-sm text-danger-text">输出读取失败，可能已过期（保留 30 天）</div>
      )}
      {exec.error && exec.status !== 'exited' && (
        <div className="text-sm text-danger-text">{exec.error}</div>
      )}
      {hasStored && exec.status !== 'running' && (output?.clipped || output?.state === 'error') && (
        <Button size="sm" variant="ghost" className="self-start" onClick={onViewOutput}>
          查看完整输出
        </Button>
      )}
    </section>
  )
}
