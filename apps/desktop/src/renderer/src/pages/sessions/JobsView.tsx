import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { CircleAlert, RefreshCw, ShieldCheck, Timer } from 'lucide-react'
import { formatAgo, formatBytes, formatDuration, formatTime } from '@/app/format'
import { approverLabel } from '@/app/policy'
import {
  Badge,
  Button,
  ConfirmDialog,
  EmptyState,
  IconButton,
  List,
  ListItem,
  Notice,
  StatusDot
} from '@/ui'
import type { Job } from '@shared/protocol/Job'
import { cancelJob, jobsStore, readJobOutput, refreshJob } from './jobs'

function jobTitle(job: Job): string {
  return job.name ?? job.command.split('\n')[0]
}

function clientLabel(job: Job): string {
  return job.client.session ? `${job.client.name} · ${job.client.session}` : job.client.name
}

export function jobBadge(job: Job) {
  switch (job.state) {
    case 'running':
      return <Badge tone="accent">运行中</Badge>
    case 'exited':
      return <Badge tone={job.exit_code === 0 ? 'success' : 'danger'}>退出 {job.exit_code}</Badge>
    case 'cancelled':
      return <Badge tone="warning">已取消</Badge>
    case 'lost':
      return <Badge tone="danger">进程丢失</Badge>
  }
}

/** E2: background jobs, newest first, with the selected job's output. Read-only except cancel. */
export function JobsView() {
  const { jobs, loaded } = jobsStore.use()
  const ordered = useMemo(
    () => Object.values(jobs).sort((a, b) => b.started_at - a.started_at),
    [jobs]
  )
  const [selected, setSelected] = useState<string | null>(null)
  const current = ordered.find((job) => job.id === selected) ?? ordered[0]

  if (loaded && ordered.length === 0) {
    return (
      <EmptyState
        icon={<Timer />}
        title="还没有后台任务"
        description="agent 用 job_start（CLI：hostlane job start）启动的长任务会显示在这里，断线后也会继续运行。"
      />
    )
  }

  return (
    <>
      <div className="flex w-64 shrink-0 flex-col overflow-y-auto border-r border-border p-2">
        <List aria-label="后台任务" value={current?.id ?? ''} onValueChange={setSelected}>
          {ordered.map((job) => (
            <ListItem
              key={job.id}
              value={job.id}
              title={jobTitle(job)}
              description={`${job.host} · ${formatAgo(job.started_at)}`}
              trailing={
                job.state === 'running' ? (
                  <span title="运行中" className="flex">
                    <StatusDot tone="accent" />
                  </span>
                ) : undefined
              }
            />
          ))}
        </List>
      </div>
      {current && <JobDetail key={current.id} job={current} />}
    </>
  )
}

function JobDetail({ job }: { job: Job }) {
  const output = jobsStore.use().outputs[job.id]
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const scroller = useRef<HTMLPreElement>(null)
  const following = useRef(true)

  useEffect(() => {
    const known = jobsStore.get().outputs[job.id]
    if (!known || known.state === 'error' || job.output_bytes > known.nextOffset) {
      void readJobOutput(job.id)
    }
  }, [job.id, job.output_bytes, job.state])

  useLayoutEffect(() => {
    const node = scroller.current
    if (node && following.current) node.scrollTop = node.scrollHeight
  }, [output?.text])

  const run = async (action: () => Promise<{ message: string } | null>) => {
    setBusy(true)
    setError(null)
    const failure = await action()
    setBusy(false)
    if (failure) setError(failure.message)
  }

  const end = job.finished_at ?? (job.state === 'running' ? Date.now() : job.checked_at)

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3 overflow-hidden p-page">
      <header className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h2 className="selectable min-w-0 flex-1 truncate text-lg font-semibold">
            {jobTitle(job)}
          </h2>
          {jobBadge(job)}
          <IconButton
            label="刷新状态"
            icon={<RefreshCw />}
            disabled={busy}
            onClick={() => void run(() => refreshJob(job.id))}
          />
          {job.state === 'running' && (
            <Button variant="danger" disabled={busy} onClick={() => setConfirming(true)}>
              取消任务
            </Button>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-fg-muted">
          <Badge>{job.host}</Badge>
          <span>{clientLabel(job)}</span>
          <span className="font-mono tabular-nums">{formatTime(job.started_at)}</span>
          <span className="tabular-nums">{formatDuration(end - job.started_at)}</span>
          <span>
            {job.backend === 'tmux' ? `tmux 会话 hl-${job.id}` : 'nohup'}
            {job.pid ? ` · pid ${job.pid}` : ''}
          </span>
          <span className="tabular-nums">{formatBytes(job.output_bytes)}</span>
          {job.approver && (
            <span className="flex items-center gap-1 text-success-text" title="审批放行">
              <ShieldCheck className="size-3.5" />
              {approverLabel(job.approver)}
            </span>
          )}
        </div>
        <pre className="selectable font-mono text-sm font-medium whitespace-pre-wrap break-all text-fg">
          <span className="text-fg-subtle select-none">$ </span>
          {job.cwd ? `cd ${job.cwd} && ` : ''}
          {job.command}
        </pre>
      </header>
      {error && (
        <Notice tone="danger" icon={<CircleAlert />} onDismiss={() => setError(null)}>
          {error}
        </Notice>
      )}
      <pre
        ref={scroller}
        onScroll={(event) => {
          const node = event.currentTarget
          following.current = node.scrollHeight - node.scrollTop - node.clientHeight < 24
        }}
        className="selectable min-h-0 flex-1 overflow-auto rounded-md border border-border bg-bg-subtle p-2 font-mono text-sm whitespace-pre-wrap break-all text-fg-muted"
      >
        {output?.clipped && <span className="text-fg-subtle">…前面的输出已省略{'\n'}</span>}
        {output?.state === 'loading' && !output.text
          ? '读取输出…'
          : output?.state === 'error' && !output.text
            ? `输出读取失败：${output.error ?? ''}`
            : output?.text || (output ? '没有输出' : '')}
      </pre>
      <p className="text-sm text-fg-subtle">
        输出来自服务器，按不可信数据显示。服务器上的完整输出在 ~/.hostlane/jobs/{job.id}/out。
      </p>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="取消这个后台任务？"
        description={`向 ${job.host} 上任务的整个进程组发送 TERM，5 秒后仍在运行则发送 KILL。`}
        confirmLabel="取消任务"
        cancelLabel="保留"
        danger
        onConfirm={() => void run(() => cancelJob(job.id))}
      />
    </div>
  )
}
