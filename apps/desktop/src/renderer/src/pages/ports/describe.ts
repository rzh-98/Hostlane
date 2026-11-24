import type { BindScope } from '@shared/protocol/BindScope'
import type { HttpProbe } from '@shared/protocol/HttpProbe'
import type { ListeningPort } from '@shared/protocol/ListeningPort'
import type { ServiceInfo } from '@shared/protocol/ServiceInfo'

export type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export const bindLabels: Record<BindScope, { label: string; tone: Tone; text: string }> = {
  loopback: { label: '仅本机', tone: 'neutral', text: '只能从服务器本机访问' },
  private: { label: '内网', tone: 'neutral', text: '绑定在内网地址上' },
  public: { label: '公网地址', tone: 'warning', text: '绑定在公网地址上，可能被外部访问' },
  all: { label: '所有网卡', tone: 'warning', text: '绑定在所有网卡上，可能被外部访问' }
}

export function statusTone(status: number): Tone {
  if (status >= 500) return 'danger'
  if (status >= 400) return 'warning'
  if (status >= 300) return 'accent'
  return 'success'
}

export function certTone(daysLeft: number): Tone {
  if (daysLeft < 0) return 'danger'
  if (daysLeft < 14) return 'warning'
  return 'neutral'
}

export function certLabel(daysLeft: number): string {
  return daysLeft < 0 ? '证书已过期' : `证书剩 ${daysLeft} 天`
}

export function serviceLabel(service: ServiceInfo): string {
  if (service.kind === 'docker') {
    const inner = service.container_port ? ` :${service.container_port}` : ''
    return `容器 ${service.name}${inner}`
  }
  return service.user_unit ? `${service.name}（用户单元）` : service.name
}

/** Who owns the port, in a few words. */
export function ownerLabel(port: ListeningPort): string {
  if (port.note?.name) return port.note.name
  if (port.service) return serviceLabel(port.service)
  if (port.process) return port.process.name
  return port.listening ? '未知进程' : '未监听'
}

const probeErrors: Record<string, string> = {
  'no HTTP response': '没有 HTTP 响应',
  'connection refused': '连接被拒绝',
  'timed out': '超时'
}

export function probeErrorLabel(error: string): string {
  if (error.startsWith('the SSH server does not allow TCP forwarding')) {
    return 'SSH 服务器不允许 TCP 转发（AllowTcpForwarding）'
  }
  return probeErrors[error] ?? error
}

export function answerLabel(port: ListeningPort): string {
  const http: HttpProbe | undefined = port.http
  if (http) {
    const title = http.title ? ` · ${http.title}` : ''
    return `${http.scheme.toUpperCase()} ${http.status}${title}`
  }
  if (port.protocol) return port.protocol.toUpperCase()
  return port.probe_error ? probeErrorLabel(port.probe_error) : ''
}
