import { useState } from 'react'
import { CircleAlert, CircleCheck, Monitor, Moon, RefreshCw, Sun } from 'lucide-react'
import { APP_NAME, APP_VERSION } from '@/app/config'
import { daemonStore, refreshDaemon, startDaemon } from '@/app/daemon'
import { runSetup, useSetup } from '@/app/setup'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  resetAppearance,
  setAppearance,
  useAppearance,
  type Density,
  type ThemePreference
} from '@/theme/appearance'
import {
  Badge,
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

const managerLabels = { launchd: 'launchd', systemd: 'systemd --user' } as const

interface SettingsNotice {
  tone: 'success' | 'danger'
  text: string
}

export function SettingsPage() {
  const appearance = useAppearance()
  const daemon = daemonStore.use()
  const setup = useSetup()
  const [confirmReset, setConfirmReset] = useState(false)
  const [confirmUninstall, setConfirmUninstall] = useState(false)
  const [notice, setNotice] = useState<SettingsNotice | null>(null)
  const status = daemon.status
  const service = setup.status?.service ?? null
  const outdated =
    service?.installed && service.binary !== null && service.binary !== setup.status?.binary

  const changeService = async (action: 'installService' | 'uninstallService') => {
    const error = await runSetup(action)
    await refreshDaemon()
    if (error) {
      setNotice({
        tone: 'danger',
        text: [error.message, error.hint].filter(Boolean).join('。')
      })
    } else {
      setNotice({
        tone: 'success',
        text:
          action === 'installService'
            ? '已注册后台服务，守护进程会在登录时启动，崩溃后自动重启。'
            : '已取消注册后台服务，守护进程已停止；点击“启动”或 agent 下次调用 MCP 时会再次启动。'
      })
    }
  }

  return (
    <Page>
      <Toolbar title="设置" />
      {notice && (
        <Notice
          tone={notice.tone}
          icon={notice.tone === 'success' ? <CircleCheck /> : <CircleAlert />}
          onDismiss={() => setNotice(null)}
        >
          {notice.text}
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
          {service?.supported && (
            <FormRow
              label="后台服务"
              description={
                outdated
                  ? `服务运行的是 ${service.binary}，与当前程序不同。`
                  : service.manager
                    ? `用 ${managerLabels[service.manager]} 在登录时启动守护进程，崩溃后自动重启。`
                    : undefined
              }
            >
              {service.installed ? (
                <span className="flex items-center gap-2">
                  <Badge tone={outdated ? 'warning' : 'success'}>已注册</Badge>
                  {outdated && (
                    <Button
                      disabled={setup.busy}
                      onClick={() => void changeService('installService')}
                    >
                      更新
                    </Button>
                  )}
                  <Button
                    className="text-danger-text"
                    disabled={setup.busy}
                    onClick={() => setConfirmUninstall(true)}
                  >
                    取消注册…
                  </Button>
                </span>
              ) : (
                <Button
                  disabled={setup.busy || !setup.status?.binaryExists}
                  onClick={() => void changeService('installService')}
                >
                  {setup.busy ? '注册中…' : '注册'}
                </Button>
              )}
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
          setNotice({ tone: 'success', text: '已恢复默认外观。' })
        }}
      />
      <ConfirmDialog
        open={confirmUninstall}
        onOpenChange={setConfirmUninstall}
        title="取消注册后台服务？"
        description="守护进程会立即停止：正在运行的命令会中断，隧道会关闭。之后登录时不再自动启动。"
        confirmLabel="取消注册"
        danger
        onConfirm={() => void changeService('uninstallService')}
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
