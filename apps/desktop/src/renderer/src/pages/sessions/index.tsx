import { SquareTerminal } from 'lucide-react'
import { definePage } from '@/app/pages'
import { SessionsPage } from './SessionsPage'

export default definePage({
  id: 'sessions',
  title: '会话',
  icon: SquareTerminal,
  order: 40,
  component: SessionsPage
})
