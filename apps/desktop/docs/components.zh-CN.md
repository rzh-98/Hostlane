# 组件 API

[English](components.md) · **简体中文**

布局组件从 `@/layout` 引入，通用组件从 `@/ui` 引入，工具函数从 `@/lib/*` 引入。除特别说明外，组件都会透传原生属性，例如 `className`、`onClick`、`aria-*`；`className` 会通过 `cn()` 合并，并能覆盖默认样式。

- [布局](#布局)：`AppShell` · `Sidebar` · `Toolbar` · `Page` / `PageContent`
- [操作](#操作)：`Button` · `IconButton` · `Menu` · `ContextMenu`
- [表单](#表单)：`Input` · `Textarea` · `Select` · `Switch` · `Checkbox` · `SegmentedControl` · `Field` · `FormRow`
- [浮层](#浮层)：`Dialog` · `ConfirmDialog` · `Tooltip`
- [数据展示](#数据展示)：`List` · `Section` · `Badge` · `StatusDot` · `Kbd`
- [反馈](#反馈)：`Notice` · `EmptyState`
- [状态与工具](#状态与工具)：`createStore` · `cn` · `platform`

## 布局

```tsx
// sidebar 可省略；Notice 可选，用于操作反馈
<AppShell sidebar={<Sidebar>…</Sidebar>}>
  <Page>
    <Toolbar title="文档" subtitle="12 个">
      <Button variant="primary">新建</Button>
    </Toolbar>
    <Notice>…</Notice>
    <PageContent>…</PageContent>
  </Page>
</AppShell>
```

| 组件          | 属性                                                                  | 说明                                                                                            |
| ------------- | --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `AppShell`    | `sidebar?`、`resizableSidebar = true`                                 | 窗口级布局。有侧栏时支持 ⌘B / Ctrl+B 折叠和拖拽调宽（双击把手可复位），状态会被记住             |
| `Sidebar`     | —                                                                     | 由 `SidebarHeader`、`SidebarContent`、`SidebarGroup label`、`SidebarItem`、`SidebarFooter` 组成 |
| `SidebarItem` | `icon?`、`active?`、`trailing?`                                       | 导航项；`active` 时使用选中样式，并设置 `aria-current="page"`                                   |
| `Toolbar`     | `title?`、`subtitle?`、`leading?`、`sidebarToggle = true`、`children` | 页面标题栏，`children` 显示在右侧。macOS 上可拖动窗口，并会为红绿灯按钮让出位置                 |
| `Page`        | —                                                                     | 纵向容器：上方是工具栏，下方内容填满剩余空间                                                    |
| `PageContent` | `width = 'default'`（`narrow` / `default` / `full`）、`padded = true` | 可滚动的内容区；`padded={false}` 适合列表加详情这类铺满布局                                     |

`useShell()` 可以读取和控制侧栏状态：`{ hasSidebar, sidebarOpen, setSidebarOpen, toggleSidebar }`。

## 操作

### Button

```tsx
<Button variant="primary" onClick={save}><Save />保存</Button>
<Button variant="ghost" size="sm">取消</Button>
<Button asChild><a href="https://example.com">文档</a></Button>
```

| 属性      | 取值                                                                  |
| --------- | --------------------------------------------------------------------- |
| `variant` | `secondary`（默认）、`primary`（每个视图只用一个）、`ghost`、`danger` |
| `size`    | `sm`、`md`（默认）、`lg`、`icon`、`icon-sm`                           |
| `asChild` | 把样式应用到子元素上，例如链接                                        |

图标作为子元素传入时，会自动设置尺寸。需要在其他元素上使用按钮样式时，可以调用 `buttonClass(variant, size)`。

### IconButton

```tsx
<IconButton label="筛选" icon={<ListFilter />} shortcut="⌘F" />
```

`label` 必填，同时用作 tooltip 和无障碍名称。其他属性包括 `shortcut?`、`size?: 'sm' | 'md'`、`tooltipSide?`。

### Menu / ContextMenu

```tsx
<Menu>
  <MenuTrigger asChild>
    <IconButton label="更多" icon={<MoreHorizontal />} />
  </MenuTrigger>
  <MenuContent align="end">
    <MenuLabel>操作</MenuLabel>
    <MenuItem icon={<Copy />} shortcut="⌘D" onSelect={duplicate}>
      创建副本
    </MenuItem>
    <MenuCheckboxItem checked={wrap} onCheckedChange={setWrap}>
      自动换行
    </MenuCheckboxItem>
    <MenuRadioGroup value={sort} onValueChange={setSort}>
      <MenuRadioItem value="name">名称</MenuRadioItem>
    </MenuRadioGroup>
    <MenuSub>
      <MenuSubTrigger>导出为</MenuSubTrigger>
      <MenuSubContent>…</MenuSubContent>
    </MenuSub>
    <MenuSeparator />
    <MenuItem danger icon={<Trash2 />}>
      删除…
    </MenuItem>
  </MenuContent>
</Menu>
```

`ContextMenu`、`ContextMenuTrigger`、`ContextMenuContent`、`ContextMenuItem`、`ContextMenuSeparator` 的写法相同，在右键或按 Shift+F10 时打开。菜单项文字以“…”结尾，表示选择后还需要进一步输入或确认。

## 表单

```tsx
<Field label="名称" description="显示在侧栏中。" error={nameError}>
  <Input value={name} onChange={(e) => setName(e.target.value)} />
</Field>

<FormRow label="主题" description="对所有页面生效。">
  <SegmentedControl value={theme} onValueChange={setTheme} options={[…]} />
</FormRow>
```

| 组件               | 主要属性                                                                                            |
| ------------------ | --------------------------------------------------------------------------------------------------- |
| `Input`            | `leading?`、`trailing?`（图标、单位、清除按钮）、`size?: 'sm' \| 'md' \| 'lg'`、`wrapperClassName?` |
| `Textarea`         | 原生 `textarea` 属性                                                                                |
| `Select`           | `options: { value, label, disabled? }[]`、`value`、`onValueChange`、`placeholder?`                  |
| `Switch`           | `checked`、`onCheckedChange`                                                                        |
| `Checkbox`         | `checked`、`onCheckedChange`、`label?`、`description?`                                              |
| `SegmentedControl` | `value`、`onValueChange`、`options: { value, label, icon?, disabled? }[]`                           |
| `Field`            | `label?`、`description?`、`error?`。纵向排列，适合对话框和表单                                      |
| `FormRow`          | `label`、`description?`。横向排列“标签 + 控件”，适合设置页和属性面板                                |

`Field` 和 `FormRow` 会通过 context 自动为子控件设置 `id`、`aria-describedby` 和 `aria-invalid`，不需要手动关联。

## 浮层

```tsx
<Dialog open={open} onOpenChange={setOpen}>
  <DialogContent size="md" closable>
    <form onSubmit={submit}>
      <DialogHeader>
        <DialogTitle>新建项目</DialogTitle>
        <DialogDescription>…</DialogDescription>
      </DialogHeader>
      <DialogBody>…</DialogBody>
      <DialogFooter>
        <DialogClose asChild>
          <Button>取消</Button>
        </DialogClose>
        <Button type="submit" variant="primary">
          创建
        </Button>
      </DialogFooter>
    </form>
  </DialogContent>
</Dialog>
```

- `DialogContent`：`size?: 'sm' | 'md' | 'lg'`、`closable?`。打开后会限制焦点，按 Esc 关闭，关闭后焦点会回到原处。
- `ConfirmDialog`：`open`、`onOpenChange`、`title`、`description?`、`confirmLabel?`、`cancelLabel?`、`danger?`、`onConfirm`、`onCloseAutoFocus?`。默认聚焦“取消”，点击遮罩不会关闭。
- `Tooltip`：`content`、`shortcut?`、`side?`。应用根节点已经包含 `TooltipProvider`。

## 数据展示

### List

单选列表，实现了 WAI-ARIA `listbox` 模式。

```tsx
<List aria-label="项目" value={selectedId} onValueChange={setSelectedId} onAction={open}>
  <ListHeader>最近</ListHeader>
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

- 键盘：↑↓ / Home / End 移动选中项；Enter 或双击触发 `onAction`。
- 列表只有一个 Tab 停靠点，并会把选中项滚动到可见区域。
- `ListItem` 可以作为 `ContextMenuTrigger` 的子元素使用。

### 其他

| 组件        | 属性                                                                 |
| ----------- | -------------------------------------------------------------------- |
| `Section`   | `title?`、`description?`、`actions?`、`divided?`（子项之间显示细线） |
| `Badge`     | `tone?: 'neutral' \| 'accent' \| 'success' \| 'warning' \| 'danger'` |
| `StatusDot` | `tone?`（取值同 `Badge`）                                            |
| `Kbd`       | 显示快捷键，例如 `<Kbd>{modKey}F</Kbd>`                              |
| `Separator` | `orientation?`                                                       |

## 反馈

### Notice

`Notice` 是放在工具栏下方的通栏提示，用于显示操作结果和可恢复的错误。它会作为 `status` 被读屏软件播报；`danger` 会作为 `alert` 播报。

```tsx
{
  lastDeleted && (
    <Notice
      tone="success"
      icon={<CircleCheck />}
      actions={
        <Button size="sm" variant="ghost" onClick={undo}>
          撤销
        </Button>
      }
      onDismiss={() => setLastDeleted(null)}
    >
      已删除“{lastDeleted.name}”。
    </Notice>
  )
}
```

| 属性        | 说明                                              |
| ----------- | ------------------------------------------------- |
| `tone`      | `neutral`（默认）、`success`、`warning`、`danger` |
| `icon`      | 前置图标，会自动使用对应状态色                    |
| `actions`   | 右侧操作，例如“撤销”或“重试”                      |
| `onDismiss` | 传入后会显示 × 关闭按钮                           |

示例：列表页删除项目后可以撤销，设置页恢复默认后会显示确认提示。

### EmptyState

`icon?`、`title`、`description?`、`actions?`。用于空列表、无搜索结果和未选择内容等状态。

## 状态与工具

### createStore

```ts
import { createStore } from '@/lib/store'

// 传入 localStorage key 会持久化；传入 null 则只在本次运行中保留。
export const editorPrefs = createStore('editor.prefs', { wrap: true, fontSize: 13 })

function Toolbar() {
  const { wrap } = editorPrefs.use() // 订阅变化并重新渲染
  return <Switch checked={wrap} onCheckedChange={(wrap) => editorPrefs.set({ wrap })} />
}

editorPrefs.set((s) => ({ fontSize: s.fontSize + 1 })) // 可以在组件外调用
editorPrefs.reset()
editorPrefs.subscribe(() => console.log(editorPrefs.get()))
```

- `set` 会浅合并状态。读取已保存的数据时，代码会与 `initial` 合并，因此新增字段可以直接得到默认值。
- 状态必须是对象；需要持久化时，值也必须能 JSON 序列化。
- 只在一个组件中使用的状态，继续用 `useState`。
- 模板中的使用位置：`theme/appearance.ts`（外观偏好）和 `pages/settings/store.ts`（示例设置）。

### 其他工具

| 导出                          | 位置             | 用途                                                     |
| ----------------------------- | ---------------- | -------------------------------------------------------- |
| `cn(...classes)`              | `@/lib/cn`       | 合并类名，并能识别自定义 token 的冲突                    |
| `isMac`、`modKey`、`isModKey` | `@/lib/platform` | 平台判断、快捷键显示和快捷键判断                         |
| `readJson`、`writeJson`       | `@/lib/storage`  | 不会抛错的 localStorage 读写                             |
| `definePage`                  | `@/app/pages`    | 声明页面，详见 [pages.ts](../src/renderer/src/app/pages.ts) |
| `navigate(id)`、`useRoute()`  | `@/app/router`   | 页面跳转、读取当前路由                                   |
