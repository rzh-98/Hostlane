import { useState } from 'react'
import { CircleCheck, Monitor, Moon, RefreshCw, Sun } from 'lucide-react'
import { APP_NAME, APP_VERSION } from '@/app/config'
import { daemonStore, refreshDaemon, startDaemon } from '@/app/daemon'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  resetAppearance,
  setAppearance,
  useAppearance,
  type Density,
  type ThemePreference
} from '@/theme/appearance'
import {
  Button,
  ConfirmDialog,
  FormRow,
  IconButton,
  Notice,
  Section,
  SegmentedControl,
  StatusDot
} from '@/ui'
import { AccentPicker } from './AccentPicker'

function formatUptime(ms: number): string {
  const minutes = Math.floor(ms / 60_000)
  if (minutes < 1) return '不到 1 分钟'
  if (minutes < 60) return `${minutes} 分钟`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时 ${minutes % 60} 分钟`
  return `${Math.floor(hours / 24)} 天 ${hours % 24} 小时`
}

export function SettingsPage() {
  const appearance = useAppearance()
  const daemon = daemonStore.use()
  const [confirmReset, setConfirmReset] = useState(false)
  const [resetDone, setResetDone] = useState(false)
  const status = daemon.status

  return (
    <Page>
      <Toolbar title="设置" />
      {resetDone && (
        <Notice tone="success" icon={<CircleCheck />} onDismiss={() => setResetDone(false)}>
          已恢复默认外观。
        </Notice>
      )}
      <PageContent width="narrow">
        <Section
          title="守护进程"
          description="守护进程常驻后台；关闭本窗口不影响 agent 使用。"
          actions={
            <IconButton label="刷新" icon={<RefreshCw />} onClick={() => void refreshDaemon()} />
          }
          divided
        >
          <FormRow label="状态">
            {status ? (
              <span className="flex items-center gap-2 text-fg-muted">
                <StatusDot tone="success" />
                运行中 · 已运行 {formatUptime(status.uptime_ms)}
              </span>
            ) : (
              <span className="flex items-center gap-3">
                <span className="flex items-center gap-2 text-fg-muted">
                  <StatusDot tone={daemon.phase === 'offline' ? 'danger' : 'neutral'} />
                  {daemon.phase === 'offline' ? '未运行' : '连接中…'}
                </span>
                {daemon.phase === 'offline' && window.desktop && (
                  <Button size="sm" variant="primary" onClick={() => void startDaemon()}>
                    启动
                  </Button>
                )}
              </span>
            )}
          </FormRow>
          {daemon.error && !status && (
            <FormRow label="错误">
              <span className="selectable text-sm text-danger-text">
                {[daemon.error.message, daemon.error.hint].filter(Boolean).join('。')}
              </span>
            </FormRow>
          )}
          {status && (
            <>
              <Mono label="版本" value={`${status.version}（协议 v${status.protocol}）`} />
              <Mono label="进程 ID" value={String(status.pid)} />
              <Mono label="Socket" value={status.socket_path} />
              <Mono label="数据目录" value={status.data_dir} />
            </>
          )}
        </Section>

        <Section title="外观" description="对所有页面立即生效。" divided>
          <FormRow label="主题">
            <SegmentedControl<ThemePreference>
              aria-label="主题"
              value={appearance.theme}
              onValueChange={(theme) => setAppearance({ theme })}
              options={[
                { value: 'light', label: '浅色', icon: <Sun /> },
                { value: 'dark', label: '深色', icon: <Moon /> },
                { value: 'system', label: '跟随系统', icon: <Monitor /> }
              ]}
            />
          </FormRow>
          <FormRow label="强调色" description="用于主要按钮、选中项和焦点。">
            <AccentPicker
              value={appearance.accentHue}
              onChange={(accentHue) => setAppearance({ accentHue })}
            />
          </FormRow>
          <FormRow label="界面密度" description="调整控件高度、行高和页面边距。">
            <SegmentedControl<Density>
              aria-label="界面密度"
              value={appearance.density}
              onValueChange={(density) => setAppearance({ density })}
              options={[
                { value: 'compact', label: '紧凑' },
                { value: 'regular', label: '标准' },
                { value: 'comfortable', label: '宽松' }
              ]}
            />
          </FormRow>
        </Section>

        <Section title="关于" divided>
          <FormRow label={APP_NAME} description={`桌面端版本 ${APP_VERSION}`}>
            <span />
          </FormRow>
          {window.desktop && (
            <FormRow label="运行环境">
              <span className="selectable text-sm text-fg-muted tabular-nums">
                Electron {window.desktop.versions.electron} · Chromium{' '}
                {window.desktop.versions.chrome.split('.')[0]} · {window.desktop.platform}
              </span>
            </FormRow>
          )}
          <FormRow label="恢复默认外观" description="重置主题、强调色和密度。">
            <Button className="text-danger-text" onClick={() => setConfirmReset(true)}>
              恢复默认…
            </Button>
          </FormRow>
        </Section>
      </PageContent>

      <ConfirmDialog
        open={confirmReset}
        onOpenChange={setConfirmReset}
        title="恢复默认外观？"
        description="主题、强调色和密度将恢复为初始值。"
        confirmLabel="恢复默认"
        danger
        onConfirm={() => {
          resetAppearance()
          setResetDone(true)
        }}
      />
    </Page>
  )
}

function Mono({ label, value }: { label: string; value: string }) {
  return (
    <FormRow label={label}>
      <span className="selectable font-mono text-sm break-all text-fg-muted">{value}</span>
    </FormRow>
  )
}
