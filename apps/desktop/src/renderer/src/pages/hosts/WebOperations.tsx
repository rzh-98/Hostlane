import { useEffect, useState } from 'react'
import { callHostlane } from '@/app/daemon'
import { Button, ConfirmDialog, Field, Input, Notice, Section, Textarea } from '@/ui'
import type { RendererMethod } from '@shared/ipc'
import type { HostNote } from '@shared/protocol/HostNote'
import type { HostProfile } from '@shared/protocol/HostProfile'
import type { WebReport } from '@shared/protocol/WebReport'
import type { HealthCheck } from '@shared/protocol/HealthCheck'
import type { ReverseForward } from '@shared/protocol/ReverseForward'

async function request<T>(method: RendererMethod, params?: unknown) {
  const reply = await callHostlane<T>(method, params)
  if (!reply.ok) throw new Error(reply.error.message)
  return reply.result
}

export function WebOperations({ host }: { host: string }) {
  const [note, setNote] = useState<string | null>(null)
  const [profile, setProfile] = useState<HostProfile | null>(null)
  const [report, setReport] = useState<WebReport | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ text: string; error?: boolean } | null>(null)
  const [checks, setChecks] = useState<HealthCheck[]>([])
  const [forwards, setForwards] = useState<ReverseForward[]>([])
  const [port, setPort] = useState('8080')
  const [remote, setRemote] = useState('0')
  const [confirm, setConfirm] = useState(false)
  useEffect(() => {
    let active = true
    async function load() {
      try {
        const [checks, routes] = await Promise.all([
          request<HealthCheck[]>('health.list'),
          request<ReverseForward[]>('reverse.list')
        ])
        if (active) {
          setChecks(checks.filter((row) => row.host === host))
          setForwards(routes.filter((row) => row.host === host))
        }
      } catch (error) {
        if (active) setMessage({ text: String(error), error: true })
      }
    }
    void load()
    const timer = window.setInterval(load, 5000)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [host])
  async function act(action: () => Promise<void>, text: string) {
    setBusy(true)
    try {
      await action()
      setMessage({ text })
    } catch (error) {
      setMessage({ text: String(error), error: true })
    } finally {
      setBusy(false)
    }
  }
  const valid = /^\d+$/.test(port) && Number(port) > 0 && Number(port) <= 65535
  const validRemote = /^\d+$/.test(remote) && Number(remote) <= 65535
  const monitor = (target: number, interval: number) =>
    act(
      async () =>
        setChecks(
          (
            await request<HealthCheck[]>('health.set', {
              host,
              port: target,
              interval_secs: interval
            })
          ).filter((row) => row.host === host)
        ),
      interval ? '已启用健康检查' : '已停止健康检查'
    )
  return (
    <Section title="Web 运维" divided>
      <div className="flex flex-col gap-3 py-2">
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={busy}
            onClick={() =>
              void act(
                async () => setProfile(await request<HostProfile>('hosts.profile', { host })),
                '已读取主机画像'
              )
            }
          >
            主机画像
          </Button>
          <Button
            disabled={busy}
            onClick={() =>
              void act(
                async () => setReport(await request<WebReport>('web.report', { host })),
                '已分析监听端口和代理配置'
              )
            }
          >
            检查暴露面与域名
          </Button>
          <Button
            disabled={busy || note !== null}
            onClick={() =>
              void act(
                async () => setNote((await request<HostNote>('hosts.export', { host })).content),
                '已载入备注'
              )
            }
          >
            编辑备注
          </Button>
          <Button
            disabled={busy}
            onClick={() =>
              void act(
                async () =>
                  navigator.clipboard.writeText(
                    (await request<HostNote>('hosts.export', { host })).markdown
                  ),
                '已复制保存的 Markdown'
              )
            }
          >
            复制 Markdown
          </Button>
        </div>
        {profile && (
          <p className="selectable">
            {profile.system} · {profile.cpu} 个逻辑 CPU · 内存 {profile.memory} · 根磁盘{' '}
            {profile.disk} · 工具：{profile.tools.join('、') || '无'}
          </p>
        )}
        {note !== null && (
          <>
            <Field label="主机备注">
              <Textarea value={note} onChange={(event) => setNote(event.target.value)} />
            </Field>
            <div>
              <Button
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await request('hosts.note', { host, content: note })
                    setNote(null)
                  }, '备注已保存')
                }
              >
                保存备注
              </Button>
            </div>
          </>
        )}
        {report && (
          <div className="selectable space-y-1 text-sm">
            <p>防火墙：{report.firewall}。此判断不含云安全组和 NAT，不等于公网实测。</p>
            {report.listeners.map((item) => (
              <p key={item.port}>
                {item.addresses.join(' / ')} · {item.port} — {item.assessment}
              </p>
            ))}
            <p>域名 → 上游</p>
            {report.routes.length ? (
              report.routes.map((route) => (
                <p key={JSON.stringify(route)}>
                  {route.domain} → {route.upstream}:{route.port} ({route.proxy})
                </p>
              ))
            ) : (
              <p>未发现可读取的静态映射</p>
            )}
          </div>
        )}
        <Field label="服务端口">
          <Input
            inputMode="numeric"
            value={port}
            onChange={(event) => setPort(event.target.value)}
          />
        </Field>
        <div>
          <Button disabled={busy || !valid} onClick={() => void monitor(Number(port), 60)}>
            每分钟检查服务器上的此端口
          </Button>
        </div>
        {checks.map((item) => (
          <div className="flex items-center justify-between" key={item.port}>
            <span>
              {item.port} · {item.detail}
            </span>
            <Button disabled={busy} onClick={() => void monitor(item.port, 0)}>
              停止检查
            </Button>
          </div>
        ))}
        <Field label="反向转发的服务器监听端口（0 自动分配）">
          <Input
            value={remote}
            inputMode="numeric"
            onChange={(event) => setRemote(event.target.value)}
          />
        </Field>
        <div>
          <Button disabled={busy || !valid || !validRemote} onClick={() => setConfirm(true)}>
            转发到本机服务端口…
          </Button>
        </div>
        {forwards.map((item) => (
          <div className="flex items-center justify-between" key={item.remote_port}>
            <span>
              服务器 {item.remote_port} → 本机 {item.local_port}
            </span>
            <Button
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  await request('reverse.close', { host, port: item.remote_port })
                  setForwards((rows) => rows.filter((row) => row.remote_port !== item.remote_port))
                }, '转发已关闭')
              }
            >
              关闭转发
            </Button>
          </div>
        ))}
        {message && (
          <Notice tone={message.error ? 'danger' : 'success'} onDismiss={() => setMessage(null)}>
            {message.text}
          </Notice>
        )}
      </div>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title="允许服务器访问本机服务？"
        description={`${host} 上的进程将能通过 SSH 访问本机 127.0.0.1:${port}。仅对可信服务器开启。`}
        confirmLabel="开启反向转发"
        danger
        onConfirm={() => {
          setConfirm(false)
          void act(async () => {
            const item = await request<ReverseForward>('reverse.open', {
              host,
              remote_port: Number(remote),
              local_port: Number(port)
            })
            setForwards((rows) => [
              ...rows.filter((row) => row.remote_port !== item.remote_port),
              item
            ])
          }, '反向转发已开启')
        }}
      />
    </Section>
  )
}
