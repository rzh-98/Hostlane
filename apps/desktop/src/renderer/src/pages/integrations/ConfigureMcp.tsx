import { useState } from 'react'
import { Button } from '@/ui'
import type { McpClient } from './snippets'

export function ConfigureMcp({ client, token }: { client: McpClient; token?: string }) {
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const configure = async () => {
    if (!window.desktop || busy) return
    setBusy(true)
    setMessage('')
    try {
      const reply = await window.desktop.setup.configureMcp(client, token)
      setMessage(
        !reply.ok
          ? reply.error.message
          : reply.result
            ? `已写入 ${reply.result}。重新打开客户端后生效。`
            : '已取消，配置未改变。'
      )
    } catch {
      setMessage('无法写入配置。请检查客户端是否已关闭和文件权限。')
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="primary" disabled={busy || !window.desktop} onClick={() => void configure()}>
        {busy ? '正在配置…' : '写入客户端配置…'}
      </Button>
      <span role="status" className="selectable break-all text-sm text-fg-muted">
        {message}
      </span>
    </div>
  )
}
