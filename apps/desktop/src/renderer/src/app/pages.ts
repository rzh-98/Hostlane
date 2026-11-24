import type { ComponentType } from 'react'

export interface PageDefinition {
  /** URL-safe id, used in the route: `#/<id>`. */
  id: string
  /** Label in the sidebar and default toolbar title. */
  title: string
  icon?: ComponentType<{ className?: string }>
  /** Sort order in the sidebar (ascending). Pages without an order come last. */
  order?: number
  /** `bottom` pins the entry to the sidebar footer (e.g. Settings). */
  placement?: 'main' | 'bottom'
  component: ComponentType
}

/** Typed helper for page modules: `export default definePage({ … })`. */
export function definePage(definition: PageDefinition): PageDefinition {
  return definition
}

/*
 * Every folder in src/renderer/src/pages with an index.tsx that default-exports
 * definePage(...) is registered automatically. Add a folder to add a page;
 * delete the folder to remove it. Nothing else needs editing.
 */
const modules = import.meta.glob<{ default: PageDefinition }>('../pages/*/index.tsx', {
  eager: true
})

export const pages: PageDefinition[] = Object.values(modules)
  .map((module) => module.default)
  .filter(Boolean)
  .sort(
    (a, b) =>
      (a.order ?? Number.MAX_SAFE_INTEGER) - (b.order ?? Number.MAX_SAFE_INTEGER) ||
      a.title.localeCompare(b.title)
  )

/** Opened at launch: the first main page, or any page if all are pinned to the bottom. */
export const defaultPage: PageDefinition | undefined =
  pages.find((page) => page.placement !== 'bottom') ?? pages[0]
