import { useEffect } from 'react'
import { LayoutTemplate } from 'lucide-react'
import { AppShell, Page, Toolbar } from '@/layout'
import { isModKey } from '@/lib/platform'
import { EmptyState, TooltipProvider } from '@/ui'
import { AppSidebar } from './AppSidebar'
import { APP_NAME } from './config'
import { defaultPage, pages } from './pages'
import { navigate, useRoute } from './router'

export function App() {
  const route = useRoute()
  const current = pages.find((page) => page.id === route) ?? defaultPage
  usePageShortcuts()

  return (
    <TooltipProvider>
      {current ? (
        <AppShell
          sidebar={<AppSidebar pages={pages} currentId={current.id} onNavigate={navigate} />}
        >
          <current.component key={current.id} />
        </AppShell>
      ) : (
        <AppShell>
          <NoPages />
        </AppShell>
      )}
    </TooltipProvider>
  )
}

/** ⌘1…⌘9 / Ctrl+1…9 jump to the main sidebar pages. */
function usePageShortcuts() {
  useEffect(() => {
    const mainPages = pages.filter((page) => page.placement !== 'bottom')
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isModKey(event) || event.shiftKey || event.altKey) return
      const index = Number(event.key) - 1
      if (Number.isInteger(index) && index >= 0 && index < mainPages.length) {
        event.preventDefault()
        navigate(mainPages[index].id)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}

/** Shown when src/renderer/src/pages is empty — the template still starts cleanly. */
function NoPages() {
  return (
    <Page>
      <Toolbar title={APP_NAME} />
      <EmptyState
        icon={<LayoutTemplate />}
        title="还没有页面"
        description={
          <>
            在 <code className="font-mono text-sm">src/renderer/src/pages/</code>{' '}
            下新建一个文件夹并添加 index.tsx，它会自动出现在侧栏中。
          </>
        }
      />
    </Page>
  )
}
