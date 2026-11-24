import { ScrollText } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PlannedFeature } from '@/app/PlannedFeature'

function AuditPage() {
  return (
    <PlannedFeature
      title="审计"
      icon={<ScrollText />}
      milestone="M1"
      summary="记录 agent 的每次调用，便于追溯和复盘。"
      items={[
        ['L1', '记录时间、来源、主机、操作、参数、结果和审批人'],
        ['L2', '按主机、来源、风险等级和时间筛选'],
        ['L3', '按任务或会话聚合成时间线'],
        ['L4', '导出 JSONL 或 CSV']
      ]}
    />
  )
}

export default definePage({
  id: 'audit',
  title: '审计',
  icon: ScrollText,
  order: 60,
  component: AuditPage
})
