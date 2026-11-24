import { Settings } from 'lucide-react'
import { definePage } from '@/app/pages'
import { SettingsPage } from './SettingsPage'

export default definePage({
  id: 'settings',
  title: '设置',
  icon: Settings,
  placement: 'bottom',
  component: SettingsPage
})
