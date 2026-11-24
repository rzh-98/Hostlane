import { ShieldCheck } from 'lucide-react'
import { definePage } from '@/app/pages'
import { ApprovalsPage } from './ApprovalsPage'

export default definePage({
  id: 'approvals',
  title: '审批',
  icon: ShieldCheck,
  order: 50,
  component: ApprovalsPage
})
