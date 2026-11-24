import { useCallback, useEffect, useState } from 'react'
import { TriangleAlert } from 'lucide-react'
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Notice,
  SegmentedControl
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { OutputChunk } from '@shared/protocol/OutputChunk'
import type { OutputStream } from '@shared/protocol/OutputStream'
import { callHostlane } from './daemon'
import { formatBytes } from './format'

const CHUNK_BYTES = 128 * 1024

interface OutputDialogProps {
  /** The exec id; the dialog is open while it is set. */
  outputId: string | null
  onClose: () => void
  title: string
}

/** Full stored stdout / stderr of one command, read page by page with `output.read`. */
export function OutputDialog({ outputId, onClose, title }: OutputDialogProps) {
  return (
    <Dialog open={outputId !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent size="lg" className="h-[calc(100vh-96px)]">
        <DialogHeader>
          <DialogTitle className="truncate font-mono">{title}</DialogTitle>
          <DialogDescription className="font-mono text-sm">{outputId}</DialogDescription>
        </DialogHeader>
        {outputId && <OutputBody key={outputId} outputId={outputId} />}
      </DialogContent>
    </Dialog>
  )
}

interface Loaded {
  data: string
  next: number
  total: number
  capped: boolean
}

function OutputBody({ outputId }: { outputId: string }) {
  const [stream, setStream] = useState<OutputStream>('stdout')
  const [loaded, setLoaded] = useState<Partial<Record<OutputStream, Loaded>>>({})
  const [error, setError] = useState<HostlaneError | null>(null)
  const [loading, setLoading] = useState(false)
  const current = loaded[stream]

  const load = useCallback(
    async (target: OutputStream, offset: number) => {
      setLoading(true)
      const reply = await callHostlane<OutputChunk>('output.read', {
        output_id: outputId,
        stream: target,
        offset,
        limit: CHUNK_BYTES
      })
      setLoading(false)
      if (!reply.ok) {
        setError(reply.error)
        return
      }
      setError(null)
      const chunk = reply.result
      setLoaded((state) => ({
        ...state,
        [target]: {
          data: (offset === 0 ? '' : (state[target]?.data ?? '')) + chunk.data,
          next: chunk.next_offset,
          total: chunk.total_bytes,
          capped: chunk.capped
        }
      }))
    },
    [outputId]
  )

  useEffect(() => {
    if (!loaded[stream]) void load(stream, 0)
  }, [stream, loaded, load])

  return (
    <>
      {error && (
        <Notice tone="danger" icon={<TriangleAlert />} className="border-t">
          {error.message}
        </Notice>
      )}
      <DialogBody className="min-h-0 flex-1 gap-2">
        <SegmentedControl
          aria-label="输出流"
          value={stream}
          onValueChange={setStream}
          className="self-start"
          options={[
            { value: 'stdout', label: 'stdout' },
            { value: 'stderr', label: 'stderr' }
          ]}
        />
        <pre className="selectable min-h-0 flex-1 overflow-auto rounded-md border border-border bg-bg-subtle p-3 font-mono text-sm whitespace-pre-wrap break-all text-fg">
          {current ? current.data || '（空）' : '读取中…'}
        </pre>
      </DialogBody>
      <DialogFooter className="justify-between">
        <span className="text-sm text-fg-muted">
          {current &&
            `已显示 ${formatBytes(current.next)} / ${formatBytes(current.total)}${current.capped ? '，超出存储上限的部分未保存' : ''}`}
        </span>
        {current && current.next < current.total && (
          <Button disabled={loading} onClick={() => void load(stream, current.next)}>
            继续加载
          </Button>
        )}
      </DialogFooter>
    </>
  )
}
