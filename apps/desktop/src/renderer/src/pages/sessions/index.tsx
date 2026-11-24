import { SquareTerminal } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PlannedFeature } from '@/app/PlannedFeature'

function SessionsPage() {
  return (
    <PlannedFeature
      title="会话"
      icon={<SquareTerminal />}
      milestone="M1"
      summary="只读监视 agent 在服务器上执行的命令和输出，不支持人工输入。"
      items={[
        ['M1', '实时显示命令、输出、退出码和耗时'],
        ['M2', '按主机或 agent 会话分标签'],
        ['M3', '回放历史会话，按命令跳转'],
        ['M4', '标记经审批放行的命令']
      ]}
    />
  )
}

export default definePage({
  id: 'sessions',
  title: '会话',
  icon: SquareTerminal,
  order: 40,
  component: SessionsPage
})
