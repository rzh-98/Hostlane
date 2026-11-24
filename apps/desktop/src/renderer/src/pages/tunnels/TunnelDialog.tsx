import { useEffect, useState } from 'react'
import {
  Button,
  Checkbox,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Field,
  Input,
  Select
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { Tunnel } from '@shared/protocol/Tunnel'

export interface TunnelDraft {
  host: string
  remotePort: number
  remoteHost: string
  /** null: pick automatically (new tunnels only). */
  localPort: number | null
  name: string
  bind: string
  allowPublic: boolean
  save: boolean
}

interface TunnelDialogProps {
  open: boolean
  /** The tunnel being edited; null creates a new one. */
  tunnel: Tunnel | null
  hosts: string[]
  defaultHost: string | null
  busy: boolean
  /** The last failed submit; PORT_IN_USE is shown next to the local port. */
  error: HostlaneError | null
  onOpenChange: (open: boolean) => void
  onSubmit: (draft: TunnelDraft) => void
}

const LOOPBACK = '127.0.0.1'
const ANY = '0.0.0.0'

function parsePort(text: string): number | null {
  const value = Number(text)
  return text.trim() !== '' && Number.isInteger(value) && value >= 1 && value <= 65535
    ? value
    : null
}

function conflictReason(message: string): string {
  if (message.includes('another tunnel')) return '已被另一条隧道使用'
  if (message.includes('administrators')) return '需要管理员权限'
  return '已被本机其他程序占用'
}

export function TunnelDialog({
  open,
  tunnel,
  hosts,
  defaultHost,
  busy,
  error,
  onOpenChange,
  onSubmit
}: TunnelDialogProps) {
  const [host, setHost] = useState('')
  const [remotePort, setRemotePort] = useState('')
  const [remoteHost, setRemoteHost] = useState(LOOPBACK)
  const [localPort, setLocalPort] = useState('')
  const [name, setName] = useState('')
  const [bind, setBind] = useState(LOOPBACK)
  const [allowPublic, setAllowPublic] = useState(false)
  const [save, setSave] = useState(true)

  useEffect(() => {
    if (!open) return
    setHost(tunnel?.host ?? defaultHost ?? hosts[0] ?? '')
    setRemotePort(tunnel ? String(tunnel.remote_port) : '')
    setRemoteHost(tunnel?.remote_host ?? LOOPBACK)
    setLocalPort(tunnel ? String(tunnel.local_port) : '')
    setName(tunnel?.name ?? '')
    setBind(tunnel?.bind ?? LOOPBACK)
    setAllowPublic(false)
    setSave(tunnel ? tunnel.saved : true)
  }, [open, tunnel, defaultHost, hosts])

  const remote = parsePort(remotePort)
  const local = parsePort(localPort)
  const publicBind = bind !== LOOPBACK && bind !== '::1'
  const bindChanged = !tunnel || tunnel.bind !== bind
  const needsConfirm = publicBind && bindChanged
  const localInvalid = localPort.trim() !== '' && local === null
  const localRequired = tunnel !== null && local === null
  const conflict = error?.kind === 'PORT_IN_USE' ? error : null
  const suggested =
    typeof conflict?.details?.suggested_port === 'number' ? conflict.details.suggested_port : null
  const valid =
    host !== '' &&
    remote !== null &&
    remoteHost.trim() !== '' &&
    !localInvalid &&
    !localRequired &&
    (!needsConfirm || allowPublic)

  const bindOptions = [
    { value: LOOPBACK, label: '仅本机（127.0.0.1）' },
    { value: ANY, label: '所有网络接口（0.0.0.0）' }
  ]
  if (!bindOptions.some((o) => o.value === bind)) bindOptions.push({ value: bind, label: bind })

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>{tunnel ? '编辑隧道' : '新建隧道'}</DialogTitle>
          <DialogDescription>把服务器能访问的地址转发到本机端口，相当于 ssh -L。</DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            if (!valid || remote === null) return
            onSubmit({
              host,
              remotePort: remote,
              remoteHost: remoteHost.trim(),
              localPort: local,
              name: name.trim(),
              bind,
              allowPublic: needsConfirm && allowPublic,
              save
            })
          }}
        >
          <DialogBody>
            <Field label="主机">
              <Select
                value={host || undefined}
                onValueChange={setHost}
                disabled={tunnel !== null}
                placeholder="选择主机"
                options={hosts.map((h) => ({ value: h, label: h }))}
              />
            </Field>
            <div className="grid grid-cols-[1fr_7rem] gap-3">
              <Field label="远程地址" description="从服务器上看到的地址">
                <Input value={remoteHost} onChange={(e) => setRemoteHost(e.target.value)} />
              </Field>
              <Field label="远程端口">
                <Input
                  inputMode="numeric"
                  placeholder="8080"
                  autoFocus={!tunnel}
                  value={remotePort}
                  onChange={(e) => setRemotePort(e.target.value)}
                />
              </Field>
            </div>
            <div className="grid grid-cols-[1fr_7rem] gap-3">
              <Field label="本机绑定">
                <Select value={bind} onValueChange={setBind} options={bindOptions} />
              </Field>
              <Field
                label="本机端口"
                error={
                  localInvalid || localRequired ? '1–65535' : conflict ? '已被占用' : undefined
                }
              >
                <Input
                  inputMode="numeric"
                  placeholder="自动"
                  value={localPort}
                  onChange={(e) => setLocalPort(e.target.value)}
                />
              </Field>
            </div>
            {conflict && (
              <p className="flex items-center justify-between gap-3 text-sm text-fg-muted">
                本机端口 {String(conflict.details?.port)} {conflictReason(conflict.message)}
                {suggested !== null ? `，可以改用 ${suggested}。` : '。'}
                {suggested !== null && (
                  <Button type="button" size="sm" onClick={() => setLocalPort(String(suggested))}>
                    改用 {suggested}
                  </Button>
                )}
              </p>
            )}
            {needsConfirm && (
              <Checkbox
                checked={allowPublic}
                onCheckedChange={(checked) => setAllowPublic(checked === true)}
                label="允许其他设备访问"
                description="同一网络中能访问这台电脑的设备都可以通过这个端口访问远程服务。"
              />
            )}
            <Field label="名称">
              <Input
                placeholder="例如 grafana"
                maxLength={100}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <Checkbox
              checked={save}
              onCheckedChange={(checked) => setSave(checked === true)}
              label="保存隧道"
              description="守护进程启动时自动建立；不保存的隧道在守护进程退出时关闭。"
            />
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button type="submit" variant="primary" disabled={!valid || busy}>
              {busy ? '处理中…' : tunnel ? '保存' : '建立隧道'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
