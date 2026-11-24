import { PlugZap } from 'lucide-react'
import { Button, EmptyState } from '@/ui'
import { daemonStore, startDaemon } from './daemon'

/** Full-page placeholder for pages that need the daemon while it is not reachable. */
export function DaemonOffline() {
  const { phase, error } = daemonStore.use()
  const starting = phase === 'starting' || phase === 'checking'
  return (
    <EmptyState
      icon={<PlugZap />}
      title={starting ? '正在连接守护进程…' : '守护进程未运行'}
      description={
        starting
          ? undefined
          : error?.kind === 'DAEMON_NOT_RUNNING'
            ? 'Hostlane 的所有能力都由后台守护进程提供。也可以在终端运行 hostlane daemon start。'
            : [error?.message, error?.hint].filter(Boolean).join('。')
      }
      actions={
        window.desktop && !starting ? (
          <Button variant="primary" onClick={() => void startDaemon()}>
            启动守护进程
          </Button>
        ) : undefined
      }
    />
  )
}
