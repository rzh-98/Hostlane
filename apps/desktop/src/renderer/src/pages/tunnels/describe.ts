import type { Tunnel } from '@shared/protocol/Tunnel'
import type { TunnelState } from '@shared/protocol/TunnelState'

export const stateLabels: Record<
  TunnelState,
  { label: string; tone: 'neutral' | 'success' | 'warning' | 'danger' }
> = {
  active: { label: '运行中', tone: 'success' },
  reconnecting: { label: '重连中', tone: 'warning' },
  failed: { label: '失败', tone: 'danger' },
  stopped: { label: '已停止', tone: 'neutral' }
}

const HTTPS_PORTS = new Set([443, 8443, 9443])

/** The address to open in a browser; well-known HTTPS ports get `https://`. */
export function browserUrl(tunnel: Tunnel, scheme?: string): string {
  const https = scheme === 'https' || (!scheme && HTTPS_PORTS.has(tunnel.remote_port))
  return https ? tunnel.url.replace(/^http:/, 'https:') : tunnel.url
}

export function tunnelTitle(tunnel: Tunnel): string {
  return tunnel.name ?? `${tunnel.host}:${tunnel.remote_port}`
}

export function remoteLabel(tunnel: Tunnel): string {
  return `${tunnel.host} → ${tunnel.remote_host}:${tunnel.remote_port}`
}

export function localLabel(tunnel: Tunnel): string {
  return `${tunnel.bind}:${tunnel.local_port}`
}

export function isRunning(tunnel: Tunnel): boolean {
  return tunnel.state === 'active' || tunnel.state === 'reconnecting'
}

/** "12 分钟后关闭"; temporary tunnels opened by agents expire. */
export function expiryLabel(expiresAt: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((expiresAt - now) / 1000))
  if (seconds < 60) return `${seconds} 秒后关闭`
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟后关闭`
  return `${Math.floor(seconds / 3600)} 小时后关闭`
}

/** The address to forward to, as seen from the server, from a port's bind addresses. */
export function remoteHostFor(addresses: string[]): string {
  if (addresses.some((a) => a === '127.0.0.1' || a === '0.0.0.0' || a === '*')) return '127.0.0.1'
  if (addresses.some((a) => a === '::1' || a === '::')) return '::1'
  return addresses[0] ?? '127.0.0.1'
}
