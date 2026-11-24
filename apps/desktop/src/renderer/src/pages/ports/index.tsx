import { Network } from 'lucide-react'
import { definePage } from '@/app/pages'
import { PortsPage } from './PortsPage'

export default definePage({
  id: 'ports',
  title: '端口',
  icon: Network,
  order: 20,
  component: PortsPage
})
