import { useEffect, useRef, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Select
} from '@/ui'
import type { ServiceCommandResult } from '@shared/protocol/ServiceCommandResult'
import type { ServiceInfo } from '@shared/protocol/ServiceInfo'

const LINE_OPTIONS = ['100', '200', '500', '2000']

export interface LogsState {
  host: string
  service: ServiceInfo
  loading: boolean
  result: ServiceCommandResult | null
  error: string | null
}

interface LogsDialogProps {
  logs: LogsState | null
  onOpenChange: (open: boolean) => void
  onLoad: (lines: number) => void
}

/** P7: the last lines of the journal or container log behind a port. */
export function LogsDialog({ logs, onOpenChange, onLoad }: LogsDialogProps) {
  const [lines, setLines] = useState('200')
  const scroller = useRef<HTMLPreElement>(null)
  const result = logs?.result
  const text = result ? result.stdout + result.stderr : ''

  useEffect(() => {
    const element = scroller.current
    if (element) element.scrollTop = element.scrollHeight
  }, [text])

  const failed = result !== null && result !== undefined && result.exit_code !== 0

  return (
    <Dialog open={logs !== null} onOpenChange={onOpenChange}>
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle>
            {logs?.service.name} 的日志
            <span className="ml-2 font-normal text-fg-muted">{logs?.host}</span>
          </DialogTitle>
          <DialogDescription className="selectable font-mono text-sm">
            {result?.command ?? (logs?.service.kind === 'docker' ? 'docker logs' : 'journalctl')}
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          {logs?.error ? (
            <p role="alert" className="text-sm text-danger-text">
              {logs.error}
            </p>
          ) : (
            <pre
              ref={scroller}
              className="selectable h-[50vh] overflow-auto rounded-md border border-border bg-bg-subtle p-2 font-mono text-sm whitespace-pre-wrap break-all text-fg"
            >
              {logs?.loading && !result ? '读取中…' : text || '没有日志'}
            </pre>
          )}
          {failed && (
            <p className="text-sm text-fg-muted">
              命令以 {result.exit_code ?? result.status} 结束
              {result.hint ? `。${result.hint}` : ''}
            </p>
          )}
          {result?.truncated && (
            <p className="text-sm text-fg-muted">输出太长，只显示开头和结尾。可以减少行数。</p>
          )}
        </DialogBody>
        <DialogFooter>
          <Select
            aria-label="行数"
            value={lines}
            onValueChange={(value) => {
              setLines(value)
              onLoad(Number(value))
            }}
            options={LINE_OPTIONS.map((value) => ({ value, label: `最后 ${value} 行` }))}
          />
          <Button disabled={logs?.loading} onClick={() => onLoad(Number(lines))}>
            <RefreshCw className={logs?.loading ? 'animate-spin' : undefined} />
            刷新
          </Button>
          <Button onClick={() => onOpenChange(false)}>关闭</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
