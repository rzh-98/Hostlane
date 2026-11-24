import type { ReactNode } from 'react'
import { Page, PageContent, Toolbar } from '@/layout'
import { Badge, EmptyState } from '@/ui'

interface PlannedFeatureProps {
  title: string
  icon: ReactNode
  milestone: string
  summary: string
  /** Requirement ids and one-line descriptions from docs/requirements.md. */
  items: [id: string, text: string][]
}

/** Placeholder for pages whose milestone has not landed yet. */
export function PlannedFeature({ title, icon, milestone, summary, items }: PlannedFeatureProps) {
  return (
    <Page>
      <Toolbar title={title} />
      <PageContent width="narrow">
        <EmptyState
          className="h-auto pt-16"
          icon={icon}
          title={
            <span className="inline-flex items-center gap-2">
              {title}
              <Badge>{milestone}</Badge>
            </span>
          }
          description={summary}
        />
        <ul className="flex flex-col divide-y divide-border border-y border-border">
          {items.map(([id, text]) => (
            <li key={id} className="flex gap-3 py-2 text-base">
              <span className="w-8 shrink-0 font-mono text-sm text-fg-subtle">{id}</span>
              <span className="text-fg-muted">{text}</span>
            </li>
          ))}
        </ul>
      </PageContent>
    </Page>
  )
}
