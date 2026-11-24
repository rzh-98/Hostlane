# 设计系统

[English](design-system.md) · **简体中文**

这份文档说明模板的视觉规则、token 和交互约定。所有数值都来自 [`src/renderer/src/styles/tokens.css`](../src/renderer/src/styles/tokens.css)，组件中不写具体色值或像素值。

- [原则](#原则)
- [Token](#token)
- [交互状态](#交互状态)
- [键盘](#键盘)
- [主题与运行时偏好](#主题与运行时偏好)
- [设计规则](#设计规则)

## 原则

1. **中性色为主**：界面中的大部分元素是灰阶。灰色带有极少量品牌色相，因此会和强调色属于同一色系。
2. **强调色有明确用途**：只用于主操作、选中项和键盘焦点。用户看到强调色，就知道“这里是重点”。
3. **用排版建立层级**：通过字号、字重、间距和 1px 分隔线区分层级。阴影只用于浮在内容上的元素，例如菜单、对话框和 tooltip。
4. **紧凑但不拥挤**：正文使用 13px / 20px；标准密度下，控件高 28px，列表行高 32px。点击区域不会因为界面紧凑而变小。

## Token

每个 token 同时也是 Tailwind 主题变量，所以可以直接写 `bg-accent`、`text-fg-muted`、`h-control`、`px-page`、`rounded-md` 这类类名。Tailwind 默认调色板、字号、圆角和阴影都已重置，只保留下列 token。

### 品牌色

| 变量             | 默认值 | 说明                                                    |
| ---------------- | ------ | ------------------------------------------------------- |
| `--brand-hue`    | `258`  | OKLCH 色相：蓝 258、紫 285、绿 160、橙 45、粉 0、青 200 |
| `--brand-chroma` | `0.17` | 饱和度；工具类产品常用 0.10–0.20                        |

`--accent-hue` 是实际生效的色相。用户在设置中选择的值（`--user-hue`）优先；没有选择时，使用 `--brand-hue`。

### 颜色

| 分组 | 类名后缀                                                                                        | 用途                                 |
| ---- | ----------------------------------------------------------------------------------------------- | ------------------------------------ |
| 表面 | `bg`、`bg-subtle`、`bg-raised`、`control`                                                       | 内容区、侧栏、浮层、输入框和次要按钮 |
| 文字 | `fg`、`fg-muted`、`fg-subtle`                                                                   | 正文、说明、占位与次要信息           |
| 线条 | `border`、`border-strong`                                                                       | 分隔线、控件边框                     |
| 交互 | `hover`、`active`                                                                               | 半透明叠加层，可以放在任意表面上     |
| 强调 | `accent`、`accent-hover`、`accent-active`、`accent-fg`、`accent-subtle`、`accent-text`、`focus` | 主按钮、选中底色、选中文字、焦点环   |
| 状态 | `danger`、`success`、`warning`，以及对应的 `-subtle`、`-text`                                   | 危险操作、状态标记、`Notice`         |

浅色值写在 `@theme static` 中，深色值写在 `[data-theme='dark']` 中，两处使用相同的变量名。

### 字号

| 类名        | 字号 / 行高 | 用途                       |
| ----------- | ----------- | -------------------------- |
| `text-xs`   | 11 / 16     | 分组标题、快捷键           |
| `text-sm`   | 12 / 16     | 说明文字、列表副标题       |
| `text-base` | 13 / 20     | 正文和控件（默认）         |
| `text-md`   | 14 / 20     | 工具栏、分组和对话框标题   |
| `text-lg`   | 16 / 24     | 页面内的大标题（按需使用） |
| `text-xl`   | 20 / 28     | 详情视图标题               |

### 尺寸与间距

| 变量                   | 标准  | 紧凑 | 宽松  | 类名示例                    |
| ---------------------- | ----- | ---- | ----- | --------------------------- |
| `--spacing`            | 4px   | —    | —     | `p-2` = 8px、`gap-3` = 12px |
| `--spacing-control-sm` | 24px  | 22px | 28px  | `h-control-sm`              |
| `--spacing-control`    | 28px  | 24px | 32px  | `h-control`、`size-control` |
| `--spacing-control-lg` | 32px  | 28px | 36px  | `h-control-lg`              |
| `--spacing-row`        | 32px  | 28px | 36px  | `min-h-row`                 |
| `--spacing-toolbar`    | 44px  | 40px | 52px  | `h-toolbar`                 |
| `--spacing-sidebar`    | 224px | —    | 240px | 侧栏默认宽度（可拖拽调整）  |
| `--spacing-page`       | 24px  | 16px | 32px  | `p-page`、`px-page`         |

宽松密度还会把正文字号提高 1px。新增密度相关的尺寸时，请先在 `@theme` 中定义默认值，再在两个 `[data-density]` 块中覆盖。

### 圆角、阴影和动效

- 圆角：`rounded-sm` 4px（徽标、复选框、菜单项）、`rounded-md` 6px（控件、列表项）、`rounded-lg` 10px（菜单、对话框）。
- 阴影：只有 `shadow-popover` 和 `shadow-dialog` 两种，并且只用于浮层。
- 动效：浮层出现时用 120–160ms 的缓出动画，消失时用 80–100ms；页面切换和列表变化不加动画。

## 交互状态

| 状态     | 表现                                             | 实现                                           |
| -------- | ------------------------------------------------ | ---------------------------------------------- |
| 悬停     | 叠加 `hover` 底色；主按钮加深一级                | `hover:bg-hover`                               |
| 按下     | 叠加 `active` 底色                               | `active:bg-active`                             |
| 选中     | `accent-subtle` 底色 + `accent-text` 文字        | 列表、侧栏、分段控件、展开的菜单按钮           |
| 菜单高亮 | 强调色填充；鼠标和键盘使用同一种样式             | `data-highlighted`                             |
| 键盘焦点 | 2px `focus` 描边，只在键盘操作时出现             | `focus-visible:focus-ring`、`focus-ring-inset` |
| 禁用     | 45% 不透明度，并且不响应指针                     | `disabled:` / `data-disabled:`                 |
| 危险     | 红色只用于确认危险操作的按钮和菜单项，不用于入口 | `variant="danger"`、`danger`                   |

`ui/` 中的组件已经实现这些状态。自己写可交互元素时，可以复用 `buttonClass()`，或参考 `ui/button.tsx`。

## 键盘

| 场景   | 按键                                                                                              |
| ------ | ------------------------------------------------------------------------------------------------- |
| 菜单   | Enter / Space / ↓ 打开；↑↓ 移动；→ 进入子菜单；← / Esc 返回；输入字母跳转；关闭后焦点回到触发按钮 |
| 对话框 | 打开后焦点进入对话框并被限制在其中；Esc 关闭；Enter 提交表单；关闭后焦点回到原处                  |
| 确认框 | 默认聚焦“取消”；点击遮罩不会关闭                                                                  |
| 列表   | ↑↓ / Home / End 移动选中项；Enter 或双击触发 `onAction`；Shift+F10 打开右键菜单                   |
| 全局   | ⌘B / Ctrl+B 切换侧栏；⌘1…9 / Ctrl+1…9 切换页面                                                    |

在界面中显示快捷键时，请使用 `modKey`（`@/lib/platform`）。它会根据平台显示 `⌘` 或 `Ctrl+`；判断快捷键时使用 `isModKey(event)`。

## 主题与运行时偏好

[`theme/appearance.ts`](../src/renderer/src/theme/appearance.ts) 管理三项偏好，并在 `<html>` 上写入对应的值：

| 偏好   | 取值                                  | 写到                             |
| ------ | ------------------------------------- | -------------------------------- |
| 主题   | `system` / `light` / `dark`           | `data-theme`，同时同步原生标题栏 |
| 密度   | `compact` / `regular` / `comfortable` | `data-density`                   |
| 强调色 | 色相数字或 `null`（使用品牌色）       | `--user-hue`                     |

```ts
import { setAppearance, resetAppearance, useAppearance } from '@/theme/appearance'

setAppearance({ theme: 'dark', density: 'compact' })
const { theme, resolvedTheme } = useAppearance() // resolvedTheme：跟随系统时实际生效的主题
```

如果产品不需要运行时切换，可以删除设置页里的相关控件，只修改 `tokens.css` 和 `defaultAppearance`。

## 设计规则

延续这套风格时请遵守：

1. 颜色、字号、间距和圆角都使用 token。需要新值时，先在 `tokens.css` 中定义语义变量。
2. 层级用字号、字重、间距和 `border` 细线表达。内容区不使用卡片和阴影。
3. 每个视图只放一个 `primary` 按钮。强调色不用于装饰。
4. 可交互元素必须具备悬停、按下、禁用和键盘焦点状态。优先使用 `ui/` 中的组件。
5. 纯图标按钮必须提供文字标签（`IconButton` 的 `label`），这个标签同时作为 tooltip 和无障碍名称。
6. 操作结果用 `Notice` 显示在工具栏下方。可以撤销的操作应提供“撤销”，而不只是询问“确定吗？”。
7. 新组件先写在页面文件夹中。第二个页面也需要它时，再去掉其中的业务概念，并提升到 `ui/`。
