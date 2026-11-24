import { ShieldCheck } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PlannedFeature } from '@/app/PlannedFeature'

function ApprovalsPage() {
  return (
    <PlannedFeature
      title="审批"
      icon={<ShieldCheck />}
      milestone="M5"
      summary="高风险操作在这里等待你确认，规则决定哪些操作需要审批。"
      items={[
        ['S1', '风险分级 L0–L3，解析管道、sh -c 和 sudo'],
        ['S2', '按主机、环境、命令、路径和来源配置规则'],
        ['S3', '阻塞等待或异步审批'],
        ['S4', '本次允许 / 本会话允许 / 总是允许'],
        ['S5', '全局急停']
      ]}
    />
  )
}

export default definePage({
  id: 'approvals',
  title: '审批',
  icon: ShieldCheck,
  order: 50,
  component: ApprovalsPage
})
