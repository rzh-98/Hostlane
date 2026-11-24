# Design system

**English** · [简体中文](design-system.zh-CN.md)

This document describes the template's visual rules, tokens and interaction conventions. Every value comes from [`src/renderer/src/styles/tokens.css`](../src/renderer/src/styles/tokens.css). Components never hard-code colors or pixel sizes.

- [Principles](#principles)
- [Tokens](#tokens)
- [Interaction states](#interaction-states)
- [Keyboard](#keyboard)
- [Theme and runtime preferences](#theme-and-runtime-preferences)
- [Design rules](#design-rules)

## Principles

1. **Mostly neutral.** Most of the interface is grayscale. The grays carry a trace of the brand hue, so they belong to the same color family as the accent.
2. **The accent has a job.** It marks primary actions, selected items and keyboard focus, and nothing else. When users see the accent color, they know it matters.
3. **Hierarchy through typography.** Type size, weight, spacing and 1px dividers separate levels. Shadows are reserved for elements that float above content, such as menus, dialogs and tooltips.
4. **Compact, not cramped.** Body text is 13px / 20px. At regular density, controls are 28px tall and list rows 32px. Click targets do not shrink just because the layout is compact.

## Tokens

Every token is also a Tailwind theme variable, so you can write classes such as `bg-accent`, `text-fg-muted`, `h-control`, `px-page` and `rounded-md` directly. Tailwind's default palette, type scale, radii and shadows are reset; only the tokens below exist.

### Brand color

| Variable         | Default | Description                                                             |
| ---------------- | ------- | ----------------------------------------------------------------------- |
| `--brand-hue`    | `258`   | OKLCH hue: blue 258, violet 285, green 160, orange 45, pink 0, teal 200 |
| `--brand-chroma` | `0.17`  | Saturation; 0.10–0.20 suits most tools                                  |

`--accent-hue` is the hue actually in use. The user's choice in Settings (`--user-hue`) takes precedence; without one, `--brand-hue` applies.

### Colors

| Group       | Class suffixes                                                                                  | Use                                                           |
| ----------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Surface     | `bg`, `bg-subtle`, `bg-raised`, `control`                                                       | Content area, sidebar, overlays, inputs and secondary buttons |
| Text        | `fg`, `fg-muted`, `fg-subtle`                                                                   | Body text, descriptions, placeholders and secondary info      |
| Line        | `border`, `border-strong`                                                                       | Dividers, control borders                                     |
| Interaction | `hover`, `active`                                                                               | Translucent layers that work on any surface                   |
| Accent      | `accent`, `accent-hover`, `accent-active`, `accent-fg`, `accent-subtle`, `accent-text`, `focus` | Primary buttons, selected background and text, focus ring     |
| Status      | `danger`, `success`, `warning`, plus their `-subtle` and `-text` variants                       | Destructive actions, status markers, `Notice`                 |

Light values live in `@theme static` and dark values in `[data-theme='dark']`, using the same variable names.

### Type scale

| Class       | Size / line height | Use                                  |
| ----------- | ------------------ | ------------------------------------ |
| `text-xs`   | 11 / 16            | Group labels, shortcuts              |
| `text-sm`   | 12 / 16            | Descriptions, list subtitles         |
| `text-base` | 13 / 20            | Body text and controls (default)     |
| `text-md`   | 14 / 20            | Toolbar, section and dialog titles   |
| `text-lg`   | 16 / 24            | Large in-page headings (when needed) |
| `text-xl`   | 20 / 28            | Detail view titles                   |

### Sizes and spacing

| Variable               | Regular | Compact | Comfortable | Class examples                    |
| ---------------------- | ------- | ------- | ----------- | --------------------------------- |
| `--spacing`            | 4px     | —       | —           | `p-2` = 8px, `gap-3` = 12px       |
| `--spacing-control-sm` | 24px    | 22px    | 28px        | `h-control-sm`                    |
| `--spacing-control`    | 28px    | 24px    | 32px        | `h-control`, `size-control`       |
| `--spacing-control-lg` | 32px    | 28px    | 36px        | `h-control-lg`                    |
| `--spacing-row`        | 32px    | 28px    | 36px        | `min-h-row`                       |
| `--spacing-toolbar`    | 44px    | 40px    | 52px        | `h-toolbar`                       |
| `--spacing-sidebar`    | 224px   | —       | 240px       | Default sidebar width (resizable) |
| `--spacing-page`       | 24px    | 16px    | 32px        | `p-page`, `px-page`               |

Comfortable density also raises body text by 1px. When you add a density-dependent size, define the default in `@theme` first, then override it in both `[data-density]` blocks.

### Radius, shadow and motion

- Radius: `rounded-sm` 4px (badges, checkboxes, menu items), `rounded-md` 6px (controls, list items), `rounded-lg` 10px (menus, dialogs).
- Shadow: only `shadow-popover` and `shadow-dialog`, and only on overlays.
- Motion: overlays enter with a 120–160ms ease-out and exit in 80–100ms. Page changes and list updates are not animated.

## Interaction states

| State          | Appearance                                                                                | Implementation                                        |
| -------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Hover          | `hover` layer; primary buttons darken one step                                            | `hover:bg-hover`                                      |
| Pressed        | `active` layer                                                                            | `active:bg-active`                                    |
| Selected       | `accent-subtle` background + `accent-text` text                                           | Lists, sidebar, segmented controls, open menu buttons |
| Menu highlight | Accent fill; mouse and keyboard look the same                                             | `data-highlighted`                                    |
| Keyboard focus | 2px `focus` outline, shown only for keyboard interaction                                  | `focus-visible:focus-ring`, `focus-ring-inset`        |
| Disabled       | 45% opacity, ignores the pointer                                                          | `disabled:` / `data-disabled:`                        |
| Destructive    | Red only on buttons and menu items that confirm a destructive action, not on entry points | `variant="danger"`, `danger`                          |

Components in `ui/` already implement these states. For your own interactive elements, reuse `buttonClass()` or follow `ui/button.tsx`.

## Keyboard

| Context | Keys                                                                                                                                    |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Menu    | Enter / Space / ↓ opens; ↑↓ moves; → opens a submenu; ← / Esc goes back; typing jumps to an item; focus returns to the trigger on close |
| Dialog  | Focus moves into the dialog and stays there; Esc closes; Enter submits the form; focus returns to where it was on close                 |
| Confirm | “Cancel” is focused by default; clicking the backdrop does not close it                                                                 |
| List    | ↑↓ / Home / End move the selection; Enter or double-click triggers `onAction`; Shift+F10 opens the context menu                         |
| Global  | ⌘B / Ctrl+B toggles the sidebar; ⌘1…9 / Ctrl+1…9 switch pages                                                                           |

To display a shortcut in the UI, use `modKey` from `@/lib/platform`; it shows `⌘` or `Ctrl+` depending on the platform. To detect one, use `isModKey(event)`.

## Theme and runtime preferences

[`theme/appearance.ts`](../src/renderer/src/theme/appearance.ts) manages three preferences and writes them to `<html>`:

| Preference | Values                                    | Written to                                       |
| ---------- | ----------------------------------------- | ------------------------------------------------ |
| Theme      | `system` / `light` / `dark`               | `data-theme`, and synced to the native title bar |
| Density    | `compact` / `regular` / `comfortable`     | `data-density`                                   |
| Accent     | A hue number, or `null` for the brand hue | `--user-hue`                                     |

```ts
import { setAppearance, resetAppearance, useAppearance } from '@/theme/appearance'

setAppearance({ theme: 'dark', density: 'compact' })
const { theme, resolvedTheme } = useAppearance() // resolvedTheme: the theme in effect when following the system
```

If your product doesn't need runtime switching, remove those controls from the settings page and only edit `tokens.css` and `defaultAppearance`.

## Design rules

Follow these rules when extending the design:

1. Use tokens for color, type size, spacing and radius. If you need a new value, define a semantic variable in `tokens.css` first.
2. Express hierarchy with type size, weight, spacing and `border` hairlines. Don't use cards or shadows in the content area.
3. Use one `primary` button per view. Don't use the accent color for decoration.
4. Every interactive element needs hover, pressed, disabled and keyboard focus states. Prefer the components in `ui/`.
5. Icon-only buttons need a text label (`IconButton`'s `label`), which also serves as the tooltip and accessible name.
6. Show action results with a `Notice` below the toolbar. For reversible actions, offer Undo instead of only asking “Are you sure?”.
7. Write new components in the page folder first. When a second page needs one, strip out the product concepts and move it to `ui/`.
