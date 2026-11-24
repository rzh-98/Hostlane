# Components

**English** · [简体中文](components.zh-CN.md)

Import layout components from `@/layout`, generic components from `@/ui`, and utilities from `@/lib/*`. Unless noted otherwise, components pass through native props such as `className`, `onClick` and `aria-*`. `className` is merged with `cn()` and can override the defaults.

- [Layout](#layout): `AppShell` · `Sidebar` · `Toolbar` · `Page` / `PageContent`
- [Actions](#actions): `Button` · `IconButton` · `Menu` · `ContextMenu`
- [Forms](#forms): `Input` · `Textarea` · `Select` · `Switch` · `Checkbox` · `SegmentedControl` · `Field` · `FormRow`
- [Overlays](#overlays): `Dialog` · `ConfirmDialog` · `Tooltip`
- [Data display](#data-display): `List` · `Section` · `Badge` · `StatusDot` · `Kbd`
- [Feedback](#feedback): `Notice` · `EmptyState`
- [State and utilities](#state-and-utilities): `createStore` · `cn` · `platform`

## Layout

```tsx
// sidebar is optional; Notice is optional, for action feedback
<AppShell sidebar={<Sidebar>…</Sidebar>}>
  <Page>
    <Toolbar title="Documents" subtitle="12 items">
      <Button variant="primary">New</Button>
    </Toolbar>
    <Notice>…</Notice>
    <PageContent>…</PageContent>
  </Page>
</AppShell>
```

| Component     | Props                                                                 | Notes                                                                                                                                                           |
| ------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `AppShell`    | `sidebar?`, `resizableSidebar = true`                                 | Window-level layout. With a sidebar, ⌘B / Ctrl+B collapses it and its edge can be dragged to resize (double-click the handle to reset). The state is remembered |
| `Sidebar`     | —                                                                     | Composed of `SidebarHeader`, `SidebarContent`, `SidebarGroup label`, `SidebarItem`, `SidebarFooter`                                                             |
| `SidebarItem` | `icon?`, `active?`, `trailing?`                                       | Navigation item; `active` applies the selected style and sets `aria-current="page"`                                                                             |
| `Toolbar`     | `title?`, `subtitle?`, `leading?`, `sidebarToggle = true`, `children` | Page title bar; `children` go on the right. On macOS it drags the window and leaves room for the traffic lights                                                 |
| `Page`        | —                                                                     | Vertical container: toolbar on top, content fills the rest                                                                                                      |
| `PageContent` | `width = 'default'` (`narrow` / `default` / `full`), `padded = true`  | Scrollable content area; `padded={false}` suits edge-to-edge layouts such as list + detail                                                                      |

`useShell()` reads and controls the sidebar: `{ hasSidebar, sidebarOpen, setSidebarOpen, toggleSidebar }`.

## Actions

### Button

```tsx
<Button variant="primary" onClick={save}><Save />Save</Button>
<Button variant="ghost" size="sm">Cancel</Button>
<Button asChild><a href="https://example.com">Docs</a></Button>
```

| Prop      | Values                                                             |
| --------- | ------------------------------------------------------------------ |
| `variant` | `secondary` (default), `primary` (one per view), `ghost`, `danger` |
| `size`    | `sm`, `md` (default), `lg`, `icon`, `icon-sm`                      |
| `asChild` | Apply the styles to the child element, such as a link              |

Icons passed as children are sized automatically. To give another element button styles, call `buttonClass(variant, size)`.

### IconButton

```tsx
<IconButton label="Filter" icon={<ListFilter />} shortcut="⌘F" />
```

`label` is required and serves as both the tooltip and the accessible name. Other props: `shortcut?`, `size?: 'sm' | 'md'`, `tooltipSide?`.

### Menu / ContextMenu

```tsx
<Menu>
  <MenuTrigger asChild>
    <IconButton label="More" icon={<MoreHorizontal />} />
  </MenuTrigger>
  <MenuContent align="end">
    <MenuLabel>Actions</MenuLabel>
    <MenuItem icon={<Copy />} shortcut="⌘D" onSelect={duplicate}>
      Duplicate
    </MenuItem>
    <MenuCheckboxItem checked={wrap} onCheckedChange={setWrap}>
      Word wrap
    </MenuCheckboxItem>
    <MenuRadioGroup value={sort} onValueChange={setSort}>
      <MenuRadioItem value="name">Name</MenuRadioItem>
    </MenuRadioGroup>
    <MenuSub>
      <MenuSubTrigger>Export as</MenuSubTrigger>
      <MenuSubContent>…</MenuSubContent>
    </MenuSub>
    <MenuSeparator />
    <MenuItem danger icon={<Trash2 />}>
      Delete…
    </MenuItem>
  </MenuContent>
</Menu>
```

`ContextMenu`, `ContextMenuTrigger`, `ContextMenuContent`, `ContextMenuItem` and `ContextMenuSeparator` work the same way and open on right-click or Shift+F10. A menu item label ending in “…” means the item needs more input or a confirmation.

## Forms

```tsx
<Field label="Name" description="Shown in the sidebar." error={nameError}>
  <Input value={name} onChange={(e) => setName(e.target.value)} />
</Field>

<FormRow label="Theme" description="Applies to every page.">
  <SegmentedControl value={theme} onValueChange={setTheme} options={[…]} />
</FormRow>
```

| Component          | Main props                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------------ |
| `Input`            | `leading?`, `trailing?` (icon, unit, clear button), `size?: 'sm' \| 'md' \| 'lg'`, `wrapperClassName?` |
| `Textarea`         | Native `textarea` props                                                                                |
| `Select`           | `options: { value, label, disabled? }[]`, `value`, `onValueChange`, `placeholder?`                     |
| `Switch`           | `checked`, `onCheckedChange`                                                                           |
| `Checkbox`         | `checked`, `onCheckedChange`, `label?`, `description?`                                                 |
| `SegmentedControl` | `value`, `onValueChange`, `options: { value, label, icon?, disabled? }[]`                              |
| `Field`            | `label?`, `description?`, `error?`. Stacked vertically; for dialogs and forms                          |
| `FormRow`          | `label`, `description?`. Label and control side by side; for settings and property panels              |

`Field` and `FormRow` use context to set `id`, `aria-describedby` and `aria-invalid` on the child control, so you don't need to wire them up yourself.

## Overlays

```tsx
<Dialog open={open} onOpenChange={setOpen}>
  <DialogContent size="md" closable>
    <form onSubmit={submit}>
      <DialogHeader>
        <DialogTitle>New project</DialogTitle>
        <DialogDescription>…</DialogDescription>
      </DialogHeader>
      <DialogBody>…</DialogBody>
      <DialogFooter>
        <DialogClose asChild>
          <Button>Cancel</Button>
        </DialogClose>
        <Button type="submit" variant="primary">
          Create
        </Button>
      </DialogFooter>
    </form>
  </DialogContent>
</Dialog>
```

- `DialogContent`: `size?: 'sm' | 'md' | 'lg'`, `closable?`. Traps focus while open, closes on Esc and returns focus to where it was.
- `ConfirmDialog`: `open`, `onOpenChange`, `title`, `description?`, `confirmLabel?`, `cancelLabel?`, `danger?`, `onConfirm`, `onCloseAutoFocus?`. Focuses “Cancel” by default; clicking the backdrop does not close it.
- `Tooltip`: `content`, `shortcut?`, `side?`. The app root already includes `TooltipProvider`.

## Data display

### List

A single-selection list that implements the WAI-ARIA `listbox` pattern.

```tsx
<List aria-label="Projects" value={selectedId} onValueChange={setSelectedId} onAction={open}>
  <ListHeader>Recent</ListHeader>
  {items.map((item) => (
    <ListItem
      key={item.id}
      value={item.id}
      leading={<FileText />}
      title={item.name}
      description={item.summary}
      trailing={<StatusDot tone="success" />}
    />
  ))}
</List>
```

- Keyboard: ↑↓ / Home / End move the selection; Enter or double-click triggers `onAction`.
- The list is a single Tab stop and scrolls the selected item into view.
- `ListItem` can be used as the child of a `ContextMenuTrigger`.

### Others

| Component   | Props                                                                         |
| ----------- | ----------------------------------------------------------------------------- |
| `Section`   | `title?`, `description?`, `actions?`, `divided?` (hairlines between children) |
| `Badge`     | `tone?: 'neutral' \| 'accent' \| 'success' \| 'warning' \| 'danger'`          |
| `StatusDot` | `tone?` (same values as `Badge`)                                              |
| `Kbd`       | Displays a shortcut, e.g. `<Kbd>{modKey}F</Kbd>`                              |
| `Separator` | `orientation?`                                                                |

## Feedback

### Notice

`Notice` is a full-width strip below the toolbar for action results and recoverable errors. Screen readers announce it as a `status`; with `danger`, it is announced as an `alert`.

```tsx
{
  lastDeleted && (
    <Notice
      tone="success"
      icon={<CircleCheck />}
      actions={
        <Button size="sm" variant="ghost" onClick={undo}>
          Undo
        </Button>
      }
      onDismiss={() => setLastDeleted(null)}
    >
      Deleted “{lastDeleted.name}”.
    </Notice>
  )
}
```

| Prop        | Description                                         |
| ----------- | --------------------------------------------------- |
| `tone`      | `neutral` (default), `success`, `warning`, `danger` |
| `icon`      | Leading icon, colored to match the tone             |
| `actions`   | Actions on the right, such as Undo or Retry         |
| `onDismiss` | When provided, shows a × close button               |

In the examples, deleting a project on the list page can be undone, and restoring defaults on the settings page shows a confirmation notice.

### EmptyState

`icon?`, `title`, `description?`, `actions?`. Use it for empty lists, no search results and nothing-selected states.

## State and utilities

### createStore

```ts
import { createStore } from '@/lib/store'

// Pass a localStorage key to persist; pass null to keep state for this session only.
export const editorPrefs = createStore('editor.prefs', { wrap: true, fontSize: 13 })

function Toolbar() {
  const { wrap } = editorPrefs.use() // subscribes and re-renders on change
  return <Switch checked={wrap} onCheckedChange={(wrap) => editorPrefs.set({ wrap })} />
}

editorPrefs.set((s) => ({ fontSize: s.fontSize + 1 })) // callable outside components
editorPrefs.reset()
editorPrefs.subscribe(() => console.log(editorPrefs.get()))
```

- `set` merges state shallowly. Saved data is merged with `initial` when it is read, so new fields get their defaults automatically.
- State must be an object, and persisted values must be JSON-serializable.
- State used by only one component should stay in `useState`.
- Used in the template by `theme/appearance.ts` (appearance preferences) and `pages/settings/store.ts` (example settings).

### Other utilities

| Export                        | Location         | Purpose                                                      |
| ----------------------------- | ---------------- | ------------------------------------------------------------ |
| `cn(...classes)`              | `@/lib/cn`       | Merge class names, resolving conflicts between custom tokens |
| `isMac`, `modKey`, `isModKey` | `@/lib/platform` | Platform detection, shortcut display and shortcut matching   |
| `readJson`, `writeJson`       | `@/lib/storage`  | localStorage reads and writes that never throw               |
| `definePage`                  | `@/app/pages`    | Declare a page; see [pages.ts](../src/renderer/src/app/pages.ts)  |
| `navigate(id)`, `useRoute()`  | `@/app/router`   | Navigate between pages, read the current route               |
