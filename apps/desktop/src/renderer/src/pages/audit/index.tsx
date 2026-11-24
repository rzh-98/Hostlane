import { ScrollText } from 'lucide-react'
import { definePage } from '@/app/pages'
import { AuditPage } from './AuditPage'

export default definePage({
  id: 'audit',
  title: '审计',
  icon: ScrollText,
  order: 60,
  component: AuditPage
})
