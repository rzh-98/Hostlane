import { Server } from 'lucide-react'
import { definePage } from '@/app/pages'
import { HostsPage } from './HostsPage'

export default definePage({
  id: 'hosts',
  title: '主机',
  icon: Server,
  order: 10,
  component: HostsPage
})
