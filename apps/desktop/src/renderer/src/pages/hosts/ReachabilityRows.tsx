import { useCallback, useEffect, useRef, useState } from 'react'
import type { Host } from '@shared/protocol/Host'
import type { HostReachability } from '@shared/protocol/HostReachability'
import { callHostlane } from '@/app/daemon'
import { formatAgo } from '@/app/format'
import { FormRow, Notice, Switch } from '@/ui'

const labels: Record<HostReachability['status'], string> = {
  pending: '等待首次检测',
  ssh_banner: 'SSH 可达（未登录）',
  session_alive: 'SSH 可达（已有连接）',
  timed_out: '本次检测超时',
  connect_failed: '本次未能连通端口',
  no_ssh_banner: '端口已接通，未收到有效 SSH 标识',
  jump_disconnected: '未检测：请先连接跳板机',
  host_unavailable: '未检测：主机配置不可用'
}

export function ReachabilityRows({ host }: { host: Host }) {
  const [sample, setSample] = useState<HostReachability | null | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const generation = useRef(0)
  const writing = useRef(false)
  const load = useCallback(async () => {
    if (writing.current) return
    const request = ++generation.current
    const reply = await callHostlane<HostReachability[]>('reachability.list')
    if (request !== generation.current) return
    if (reply.ok) {
      setSample(reply.result.find((record) => record.host === host.id) ?? null)
      setError('')
    } else
      setError(
        reply.error.kind === 'METHOD_NOT_FOUND'
          ? '请更新并重启 Hostlane 守护进程以使用后台检测。'
          : reply.error.message
      )
  }, [host.id])
  useEffect(() => {
    void load()
    const timer = window.setInterval(load, 5000)
    window.addEventListener('focus', load)
    return () => {
      generation.current++
      window.clearInterval(timer)
      window.removeEventListener('focus', load)
    }
  }, [load])
  async function toggle(enabled: boolean) {
    const request = ++generation.current
    writing.current = true
    setBusy(true)
    try {
      const reply = await callHostlane('reachability.set', { host: host.id, enabled })
      if (request !== generation.current) return
      if (!reply.ok) {
        setError(reply.error.message)
        return
      }
      setError('')
      setSample(enabled ? { host: host.id, status: 'pending', checked_at: 0 } : null)
    } finally {
      writing.current = false
      setBusy(false)
    }
  }
  const stale = sample && sample.checked_at > 0 && Date.now() - sample.checked_at > 300_000
  return (
    <>
      <FormRow
        label="后台检测 SSH 可达性"
        description={
          host.proxy_jump
            ? '每 5 分钟检测；仅复用已有 SSH 或跳板连接，不自动登录。'
            : '每 5 分钟检测，不登录；关闭桌面窗口后守护进程仍会继续。'
        }
      >
        <Switch
          checked={!!sample}
          disabled={busy || sample === undefined}
          onCheckedChange={toggle}
        />
      </FormRow>
      {sample && (
        <FormRow
          label="最近检测"
          description={
            sample.checked_at
              ? `${formatAgo(sample.checked_at)}${stale ? ' · 结果已过期，等待新样本' : ''}`
              : undefined
          }
        >
          <span className="text-sm text-fg-muted">{labels[sample.status]}</span>
        </FormRow>
      )}
      {error && <Notice tone="warning">{error}</Notice>}
    </>
  )
}
