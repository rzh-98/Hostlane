import { useCallback, useEffect, useState } from 'react'
import { CircleCheck, OctagonPause, ShieldCheck, TriangleAlert } from 'lucide-react'
import { callHostlane, daemonStore } from '@/app/daemon'
import { DaemonOffline } from '@/app/DaemonOffline'
import { useHostlaneEvent } from '@/app/events'
import { formatAgo } from '@/app/format'
import { policyStore, putApproval, resumeAgents, riskLabels } from '@/app/policy'
import { Page, PageContent, Toolbar } from '@/layout'
import {
  Badge,
  Button,
  ConfirmDialog,
  EmptyState,
  List,
  ListItem,
  Notice,
  SegmentedControl
} from '@/ui'
import type { HostlaneError } from '@shared/ipc'
import type { Approval } from '@shared/protocol/Approval'
import type { ApprovalScope } from '@shared/protocol/ApprovalScope'
import type { HostsList } from '@shared/protocol/HostsList'
import type { PolicyRule } from '@shared/protocol/PolicyRule'
import type { PolicyRuleAddParams } from '@shared/protocol/PolicyRuleAddParams'
import type { PolicyRules } from '@shared/protocol/PolicyRules'
import { ApprovalDetail } from './ApprovalDetail'
import { DenyDialog } from './DenyDialog'
import { describeConditions, scopeLabels, stateBadge } from './describe'
import { RuleDialog } from './RuleDialog'
import { RulesView } from './RulesView'

type View = 'approvals' | 'rules'

interface Feedback {
  tone: 'success' | 'warning' | 'danger'
  text: string
  hint?: string
  /** Undo for "always allow": deletes the rule it made. */
  undoRule?: string
}

function errorFeedback(error: HostlaneError): Feedback {
  return { tone: 'danger', text: error.message, hint: error.hint }
}

export function ApprovalsPage() {
  const { phase } = daemonStore.use()
  const online = phase === 'online'
  const { status, approvals, notice } = policyStore.use()
  const [view, setView] = useState<View>('approvals')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [busy, setBusy] = useState(false)
  const [denying, setDenying] = useState<Approval | null>(null)
  const [resuming, setResuming] = useState(false)
  const [rules, setRules] = useState<PolicyRule[] | null>(null)
  const [hosts, setHosts] = useState<string[]>([])
  const [adding, setAdding] = useState(false)
  const [deleting, setDeleting] = useState<PolicyRule | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const loadRules = useCallback(async () => {
    const reply = await callHostlane<PolicyRules>('policy.rules.list')
    if (reply.ok) setRules(reply.result.rules)
    else setFeedback(errorFeedback(reply.error))
  }, [])

  useEffect(() => {
    if (!online) return
    void loadRules()
    void callHostlane<HostsList>('hosts.list').then((reply) => {
      if (reply.ok) setHosts(reply.result.hosts.map((h) => h.id))
    })
  }, [online, loadRules])

  useEffect(() => {
    if (notice) setFeedback(null)
  }, [notice])
  useEffect(() => {
    if (feedback) policyStore.set({ notice: null })
  }, [feedback])

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 10_000)
    return () => clearInterval(timer)
  }, [])

  useHostlaneEvent((event) => {
    if (event.type === 'subscribed' || event.type === 'lagged') void loadRules()
    else if (event.type === 'audit_appended' && event.entry.method.startsWith('policy.rules.')) {
      void loadRules()
    } else if (event.type === 'approval_updated' && event.approval.scope === 'always') {
      void loadRules()
    }
  })

  const list = approvals ?? []
  const pending = list.filter((a) => a.state === 'pending')
  const selected = list.find((a) => a.id === selectedId) ?? pending[0] ?? list[0] ?? null

  const resolve = async (
    approval: Approval,
    approve: boolean,
    scope?: ApprovalScope,
    note?: string
  ) => {
    setBusy(true)
    setFeedback(null)
    const reply = await callHostlane<Approval>('approvals.resolve', {
      id: approval.id,
      approve,
      scope,
      note: note || undefined
    })
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    putApproval(reply.result)
    const next = pending.find((a) => a.id !== approval.id)
    setSelectedId(next ? next.id : reply.result.id)
    if (!approve) {
      setFeedback({ tone: 'success', text: `已拒绝 ${approval.summary}` })
      return
    }
    let undoRule: string | undefined
    if (scope === 'always') {
      const rulesReply = await callHostlane<PolicyRules>('policy.rules.list')
      if (rulesReply.ok) {
        setRules(rulesReply.result.rules)
        undoRule = rulesReply.result.rules.find(
          (rule) => rule.created_by === `approval ${approval.id}`
        )?.id
      }
    }
    setFeedback({
      tone: 'success',
      text: `${scopeLabels[scope ?? 'once']}：${approval.summary}`,
      hint: scope === 'always' ? '已添加一条允许规则' : undefined,
      undoRule
    })
  }

  const addRule = async (rule: PolicyRuleAddParams) => {
    setBusy(true)
    const reply = await callHostlane<PolicyRule>('policy.rules.add', rule)
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    setAdding(false)
    setRules((current) => [...(current ?? []), reply.result])
    setFeedback({ tone: 'success', text: `已添加规则：${describeConditions(reply.result.when)}` })
  }

  const deleteRule = async (id: string, undo: boolean) => {
    setBusy(true)
    const reply = await callHostlane('policy.rules.delete', { id })
    setBusy(false)
    if (!reply.ok) {
      setFeedback(errorFeedback(reply.error))
      return
    }
    setRules((current) => current?.filter((rule) => rule.id !== id) ?? current)
    setFeedback({ tone: 'success', text: undo ? '已撤销，删除了刚添加的规则' : '已删除规则' })
  }

  const shownNotice = feedback ?? notice

  return (
    <Page>
      <Toolbar
        title="审批"
        subtitle={pending.length > 0 ? `${pending.length} 个待处理` : undefined}
      >
        <SegmentedControl
          aria-label="视图"
          value={view}
          onValueChange={setView}
          options={[
            { value: 'approvals', label: '审批' },
            { value: 'rules', label: '规则' }
          ]}
        />
        {view === 'rules' && (
          <Button variant="primary" disabled={!online} onClick={() => setAdding(true)}>
            添加规则
          </Button>
        )}
      </Toolbar>

      {online && status?.stopped && (
        <Notice
          tone="danger"
          icon={<OctagonPause />}
          actions={
            <Button size="sm" onClick={() => setResuming(true)}>
              恢复
            </Button>
          }
        >
          急停已开启{status.reason ? `（${status.reason}）` : ''}：agent 的调用都会被拒绝。
          {status.stopped_by && (
            <span className="text-fg-muted">
              {' '}
              {status.stopped_by} · {status.stopped_at ? formatAgo(status.stopped_at, now) : ''}
            </span>
          )}
        </Notice>
      )}

      {online && shownNotice && (
        <Notice
          tone={shownNotice.tone}
          icon={shownNotice.tone === 'success' ? <CircleCheck /> : <TriangleAlert />}
          actions={
            feedback?.undoRule ? (
              <Button
                size="sm"
                disabled={busy}
                onClick={() => feedback.undoRule && void deleteRule(feedback.undoRule, true)}
              >
                撤销
              </Button>
            ) : undefined
          }
          onDismiss={() => (feedback ? setFeedback(null) : policyStore.set({ notice: null }))}
        >
          <span className="selectable break-all">{shownNotice.text}</span>
          {shownNotice.hint && <span className="text-fg-muted">。{shownNotice.hint}</span>}
        </Notice>
      )}

      {!online ? (
        <DaemonOffline />
      ) : view === 'rules' ? (
        <PageContent padded={false}>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {rules && <RulesView rules={rules} now={now} busy={busy} onDelete={setDeleting} />}
          </div>
        </PageContent>
      ) : !approvals ? (
        <EmptyState icon={<ShieldCheck />} title="正在加载审批…" />
      ) : approvals.length === 0 ? (
        <EmptyState
          icon={<ShieldCheck />}
          title="没有审批"
          description="agent 的操作需要确认时会出现在这里，同时发送系统通知。哪些操作需要审批由规则和风险等级决定。"
        />
      ) : (
        <PageContent padded={false}>
          <div className="flex w-[24rem] shrink-0 flex-col overflow-y-auto border-r border-border p-2">
            <List aria-label="审批列表" value={selected?.id ?? null} onValueChange={setSelectedId}>
              {list.map((approval) => (
                <ApprovalItem key={approval.id} approval={approval} now={now} />
              ))}
            </List>
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto">
            {selected && (
              <ApprovalDetail
                key={selected.id}
                approval={selected}
                now={now}
                busy={busy}
                onApprove={(scope) => void resolve(selected, true, scope)}
                onDeny={() => setDenying(selected)}
              />
            )}
          </div>
        </PageContent>
      )}

      <DenyDialog
        approval={denying}
        onOpenChange={(open) => !open && setDenying(null)}
        onDeny={(note) => {
          if (denying) void resolve(denying, false, undefined, note)
          setDenying(null)
        }}
      />
      <RuleDialog
        open={adding}
        hosts={hosts}
        busy={busy}
        onOpenChange={setAdding}
        onSubmit={(rule) => void addRule(rule)}
      />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title="删除这条规则？"
        description={deleting ? describeConditions(deleting.when) : undefined}
        confirmLabel="删除"
        danger
        onConfirm={() => deleting && void deleteRule(deleting.id, false)}
      />
      <ConfirmDialog
        open={resuming}
        onOpenChange={setResuming}
        title="恢复 agent 操作？"
        description="agent 的调用会重新按策略处理。急停期间关闭的隧道不会自动恢复。"
        confirmLabel="恢复"
        onConfirm={() => void resumeAgents()}
      />
    </Page>
  )
}

function ApprovalItem({ approval, now }: { approval: Approval; now: number }) {
  const risk = riskLabels[approval.risk]
  const state = stateBadge(approval)
  return (
    <ListItem
      value={approval.id}
      title={<span className="truncate font-mono text-sm">{approval.summary}</span>}
      description={[approval.client.name, approval.host, formatAgo(approval.created_at, now)]
        .filter(Boolean)
        .join(' · ')}
      trailing={
        <>
          <Badge tone={risk.tone}>{approval.risk}</Badge>
          {approval.state !== 'pending' || approval.waiting ? (
            <Badge tone={state.tone}>{state.label}</Badge>
          ) : null}
        </>
      }
    />
  )
}
