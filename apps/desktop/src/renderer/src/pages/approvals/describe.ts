import { approverLabel } from '@/app/policy'
import type { Approval } from '@shared/protocol/Approval'
import type { ApprovalScope } from '@shared/protocol/ApprovalScope'
import type { Decision } from '@shared/protocol/Decision'
import type { RuleConditions } from '@shared/protocol/RuleConditions'

type Tone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger'

export function stateBadge(approval: Approval): { label: string; tone: Tone } {
  switch (approval.state) {
    case 'pending':
      return approval.waiting
        ? { label: 'agent 等待中', tone: 'accent' }
        : { label: '待审批', tone: 'warning' }
    case 'approved':
      return {
        label: approval.used || approval.scope !== 'once' ? '已允许' : '已允许 · 未执行',
        tone: 'success'
      }
    case 'denied':
      return { label: '已拒绝', tone: 'danger' }
    case 'expired':
      return { label: '已过期', tone: 'neutral' }
  }
}

export const scopeLabels: Record<ApprovalScope, string> = {
  once: '本次允许',
  session: '本会话允许',
  always: '总是允许'
}

export const decisionLabels: Record<Decision, { label: string; tone: Tone }> = {
  allow: { label: '允许', tone: 'success' },
  approve: { label: '需要审批', tone: 'warning' },
  deny: { label: '拒绝', tone: 'danger' }
}

/** What asked for approval: `default` or `rule <id>`. */
export function matchedLabel(matched: string): string {
  if (matched === 'default') return '默认规则'
  return approverLabel(matched)
}

const conditionLabels: [keyof RuleConditions, string][] = [
  ['host', '主机'],
  ['env', '环境'],
  ['tag', '标签'],
  ['client', '来源'],
  ['method', '方法'],
  ['command', '命令'],
  ['path', '路径'],
  ['min_risk', '风险 ≥'],
  ['max_risk', '风险 ≤']
]

/** `主机 web-* · 命令 systemctl restart *`, or 所有调用 when no condition is set. */
export function describeConditions(when: RuleConditions): string {
  const parts = conditionLabels
    .filter(([key]) => when[key] !== undefined && when[key] !== null)
    .map(([key, label]) => `${label} ${String(when[key])}`)
  return parts.length > 0 ? parts.join(' · ') : '所有调用'
}

/** Approver, for decided approvals: `desktop · 本会话允许`. */
export function resolutionLabel(approval: Approval): string {
  const parts = [approval.resolved_by ?? '—']
  if (approval.state === 'approved' && approval.scope) parts.push(scopeLabels[approval.scope])
  return parts.join(' · ')
}
