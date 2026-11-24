import { ArrowLeftRight } from 'lucide-react'
import { definePage } from '@/app/pages'
import { TunnelsPage } from './TunnelsPage'

export default definePage({
  id: 'tunnels',
  title: '隧道',
  icon: ArrowLeftRight,
  order: 30,
  component: TunnelsPage
})
