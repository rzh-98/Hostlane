import { Cable } from 'lucide-react'
import { definePage } from '@/app/pages'
import { IntegrationsPage } from './IntegrationsPage'

export default definePage({
  id: 'integrations',
  title: '接入',
  icon: Cable,
  order: 70,
  component: IntegrationsPage
})
