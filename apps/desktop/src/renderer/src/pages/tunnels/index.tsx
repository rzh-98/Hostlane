import { ArrowLeftRight } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PlannedFeature } from '@/app/PlannedFeature'

function TunnelsPage() {
  return (
    <PlannedFeature
      title="隧道"
      icon={<ArrowLeftRight />}
      milestone="M4"
      summary="把服务器上的端口映射到本机，或把本机端口暴露给服务器。"
      items={[
        ['T1', '本地转发：本机端口 → 远程地址:端口'],
        ['T4', '隧道状态、活跃连接数和收发流量'],
        ['T5', '保存配置，随守护进程自动建立，断线自动重连'],
        ['T6', '检测本机端口冲突并推荐空闲端口'],
        ['T7', '默认只绑定 127.0.0.1'],
        ['T8', '一键在浏览器中打开'],
        ['T9', 'agent 创建的临时隧道到期自动关闭']
      ]}
    />
  )
}

export default definePage({
  id: 'tunnels',
  title: '隧道',
  icon: ArrowLeftRight,
  order: 30,
  component: TunnelsPage
})
