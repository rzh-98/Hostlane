import { useEffect } from 'react'
import { createStore } from '@/lib/store'
import type { HostlaneReply } from '@shared/ipc'
import type { Approval } from '@shared/protocol/Approval'
import type { ApprovalsList } from '@shared/protocol/ApprovalsList'
import type { PolicyStatus } from '@shared/protocol/PolicyStatus'
import type { PolicyStopped } from '@shared/protocol/PolicyStopped'
import type { Risk } from '@shared/protocol/Risk'
import { callHostlane, daemonStore } from './daemon'
import { useHostlaneEvent } from './events'

type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export const riskLabels: Record<Risk, { label: string; tone: Tone }> = {
  L0: { label: 'L0 只读', tone: 'neutral' },
  L1: { label: 'L1 用户态修改', tone: 'accent' },
  L2: { label: 'L2 系统变更', tone: 'warning' },
  L3: { label: 'L3 破坏性', tone: 'danger' }
}

/** Who let a call through, as stored in `approver`. */
export function approverLabel(approver: string): string {
  if (approver === 'session grant') return '本会话授权'
  if (approver.startsWith('rule ')) return `规则 ${approver.slice(5)}`
  return approver
}

export interface PolicyNotice {
  tone: 'success' | 'warning' | 'danger'
  text: string
  hint?: string
}

interface PolicyState {
  status: PolicyStatus | null
  /** Result of the last stop or resume, shown on the approvals page. */
  notice: PolicyNotice | null
  /** Pending first (oldest first), then decided ones (newest first), like `approvals.list`. */
  approvals: Approval[] | null
}

/** Emergency stop and approval queue, kept current by daemon events. */
export const policyStore = createStore<PolicyState>(null, {
  status: null,
  notice: null,
  approvals: null
})

function sorted(approvals: Approval[]): Approval[] {
  const pending = approvals
    .filter((a) => a.state === 'pending')
    .sort((a, b) => a.created_at - b.created_at)
  const decided = approvals
    .filter((a) => a.state !== 'pending')
    .sort((a, b) => (b.resolved_at ?? b.created_at) - (a.resolved_at ?? a.created_at))
  return [...pending, ...decided]
}

export function putApproval(approval: Approval): void {
  policyStore.set((state) => ({
    approvals: state.approvals
      ? sorted([...state.approvals.filter((a) => a.id !== approval.id), approval])
      : state.approvals
  }))
}

export async function loadPolicy(): Promise<void> {
  const [status, list] = await Promise.all([
    callHostlane<PolicyStatus>('policy.status'),
    callHostlane<ApprovalsList>('approvals.list')
  ])
  policyStore.set({
    status: status.ok ? status.result : null,
    approvals: list.ok ? list.result.approvals : null
  })
}

export async function emergencyStop(reason?: string): Promise<HostlaneReply<PolicyStopped>> {
  const reply = await callHostlane<PolicyStopped>('policy.stop', { reason })
  if (reply.ok) {
    const { status, closed_tunnels, denied_approvals, cancelled_jobs } = reply.result
    const done = [
      closed_tunnels.length > 0 ? `关闭了 ${closed_tunnels.length} 条 agent 隧道` : null,
      cancelled_jobs.length > 0 ? `取消了 ${cancelled_jobs.length} 个后台任务` : null,
      denied_approvals.length > 0 ? `拒绝了 ${denied_approvals.length} 个待审批` : null
    ].filter(Boolean)
    policyStore.set({
      status,
      notice: {
        tone: 'warning',
        text: `已急停${done.length > 0 ? `，${done.join('，')}` : ''}`
      }
    })
  } else {
    policyStore.set({
      notice: { tone: 'danger', text: reply.error.message, hint: reply.error.hint }
    })
  }
  return reply
}

export async function resumeAgents(): Promise<HostlaneReply<PolicyStatus>> {
  const reply = await callHostlane<PolicyStatus>('policy.resume')
  policyStore.set(
    reply.ok
      ? {
          status: reply.result,
          notice: { tone: 'success', text: '已恢复，agent 的调用重新按策略处理' }
        }
      : { notice: { tone: 'danger', text: reply.error.message, hint: reply.error.hint } }
  )
  return reply
}

/** Mount once, near the root. */
export function usePolicySync(): void {
  const { phase } = daemonStore.use()
  useEffect(() => {
    if (phase === 'online') void loadPolicy()
    else if (phase === 'offline') policyStore.reset()
  }, [phase])
  useHostlaneEvent((event) => {
    if (event.type === 'policy') policyStore.set({ status: event.status })
    else if (event.type === 'approval_requested' || event.type === 'approval_updated') {
      putApproval(event.approval)
    } else if (event.type === 'subscribed' || event.type === 'lagged') void loadPolicy()
  })
}
