import { Network } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PlannedFeature } from '@/app/PlannedFeature'

function PortsPage() {
  return (
    <PlannedFeature
      title="端口"
      icon={<Network />}
      milestone="M3"
      summary="查看各主机上的监听端口、对应的进程和服务，以及 web 服务的状态。"
      items={[
        ['P1', '通过 ss / netstat / lsof 发现监听端口'],
        ['P2', '关联进程：PID、命令行、运行用户、启动时间'],
        ['P3', '关联 systemd 单元、Docker 容器与 compose 项目'],
        ['P4', 'HTTP/HTTPS 探测：状态码、标题、Server 头、证书有效期'],
        ['P7', '快捷操作：转发并打开、查看日志、重启或停止服务'],
        ['P8', '端口备注：命名并关联项目'],
        ['P10', '检查端口占用、查找空闲端口']
      ]}
    />
  )
}

export default definePage({
  id: 'ports',
  title: '端口',
  icon: Network,
  order: 20,
  component: PortsPage
})
