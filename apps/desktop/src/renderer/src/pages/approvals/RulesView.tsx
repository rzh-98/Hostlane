import { Trash2 } from 'lucide-react'
import { formatAgo } from '@/app/format'
import { IconButton, Badge } from '@/ui'
import type { PolicyRule } from '@shared/protocol/PolicyRule'
import { decisionLabels, describeConditions } from './describe'

interface RulesViewProps {
  rules: PolicyRule[]
  now: number
  busy: boolean
  onDelete: (rule: PolicyRule) => void
}

const DEFAULTS: [string, string][] = [
  ['L0 只读', '允许'],
  ['L1 用户态修改', '允许；prod 主机需要审批'],
  ['L2 系统变更', '需要审批'],
  ['L3 破坏性', '需要审批；prod 主机直接拒绝']
]

/** User rules, then the built-in defaults for calls no rule matches. */
export function RulesView({ rules, now, busy, onDelete }: RulesViewProps) {
  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-8 p-page">
      <section className="flex flex-col gap-2">
        <h3 className="text-md font-semibold">规则</h3>
        <p className="text-sm text-fg-muted">
          多条规则同时匹配时，拒绝优先于允许，允许优先于需要审批。桌面端自己的操作不受规则限制。
        </p>
        {rules.length === 0 ? (
          <p className="py-4 text-base text-fg-muted">
            还没有规则。审批时选择“总是允许”也会在这里生成一条规则。
          </p>
        ) : (
          <ul className="flex flex-col divide-y divide-border border-y border-border">
            {rules.map((rule) => {
              const decision = decisionLabels[rule.decision]
              return (
                <li key={rule.id} className="flex min-h-row items-center gap-3 py-2">
                  <Badge tone={decision.tone} className="w-16 justify-center">
                    {decision.label}
                  </Badge>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="selectable font-mono text-sm break-all text-fg">
                      {describeConditions(rule.when)}
                    </span>
                    <span className="truncate text-sm text-fg-muted">
                      {[rule.note, rule.created_by, formatAgo(rule.created_at, now)]
                        .filter(Boolean)
                        .join(' · ')}
                    </span>
                  </div>
                  <IconButton
                    label="删除规则"
                    icon={<Trash2 />}
                    disabled={busy}
                    onClick={() => onDelete(rule)}
                  />
                </li>
              )
            })}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <h3 className="text-md font-semibold">默认</h3>
        <p className="text-sm text-fg-muted">没有规则匹配时，按风险等级和主机环境决定。</p>
        <ul className="flex flex-col divide-y divide-border border-y border-border">
          {DEFAULTS.map(([risk, decision]) => (
            <li key={risk} className="flex min-h-row items-center justify-between gap-6 py-2">
              <span className="text-base text-fg">{risk}</span>
              <span className="text-sm text-fg-muted">{decision}</span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  )
}
