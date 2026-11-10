# Design System — NyxID

## Product Context
- **What this is:** Auth/SSO platform with credential management, proxy, AI agent isolation, and approval workflows
- **Who it's for:** Developers, DevOps engineers, security engineers managing AI agent access to external services
- **Space/industry:** Identity & access management (peers: Clerk, Auth0, WorkOS, Stytch)
- **Project type:** SaaS dashboard / developer tool

## Aesthetic Direction
- **Direction:** Refined dark mode with warm purple identity
- **Decoration level:** Intentional — purple used sparingly on active states, borders, badges, logo. Not on every surface or hover.
- **Mood:** Night goddess energy. Precise, confident, measured use of color. Purple as identity, not wallpaper. The kind of product a security team trusts because it looks like it takes itself seriously.
- **Key principle:** Color is earned. Semantic colors (green/amber/red) do the heavy lifting for status. Purple marks identity, interaction, and the "primary" CTA archetype.
- **Density:** Compact — the entire UI skews small and tight. Elements are visually cohesive at reduced sizes. If in doubt, go smaller.

## Color

### Primary Accent
- **Primary:** `#9775fa` — warm violet (NOT #8b5cf6 which is the AI-default Tailwind violet-500)
- **Primary light:** `#c4b5fd` — for logo wordmark, light text on accent backgrounds
- **Primary deep:** `#7c5ce0` — for hover states, pressed states

### Themes and tokens
The dashboard has dark and light themes (`html.theme-dark` / `html.theme-light`, chosen in Settings → Display; default follows the OS). Always use the semantic tokens below, never raw hex or `white/[α]` classes, so both themes stay correct. Source of truth: `frontend/src/app.css`. Landing/blog are never themed and keep the `@theme` defaults.

| Token | Dark | Light | Use |
|---|---|---|---|
| `bg-background` | `#17171A` | `#F4F4F5` | Page canvas and header |
| `bg-sidebar` | `#1C1C1F` | `#FFFFFF` | Assistant window surface: the sidebar column and the gutter around the content slab (light: the card white). The assistant content area is one `bg-background` slab (`rounded-2xl`, inset 8px, `shadow-sm`) with no divider lines; chat and the machine column share the slab side by side, flush, with nothing drawn between them |
| `bg-sidebar-hover` | `#27272A` | `#F6F6F6` | Opaque sidebar row hover (`bg-overlay` over `bg-sidebar`), so avatar and status-dot rings can match the hovered row |
| `bg-card` / `bg-popover` | `#222225` | `#FFFFFF` | Elevated content (cards carry `shadow-sm`) |
| `bg-muted` | `#2C2C30` | `#F4F4F5` | Neutral fills, chips |
| `text-foreground` | `#EFEFF1` | `#18181B` | Headings, primary text |
| `text-muted-foreground` | `#BABAC1` | `#3F3F46` | Body, descriptions |
| `text-text-tertiary` | `#95959F` | `#5B5B63` | Timestamps, placeholders, section labels (≥ 4.5:1) |
| `border-border` | `#414148` | `#C4C4CA` | Card borders, dividers |
| `border-input` | `#55555E` | `#8E8E96` | Control edges (inputs, selects) |
| `border-input-focus` | `#B0B0B8` | `#7C5CF0` | Focused control edge |
| `border-hairline` / `-strong` | white 14% / 24% | black 20% / 34% | Interactive chrome idle / hover |
| `bg-overlay` / `-strong` | white 5% / 9.5% | black 3.5% / 6% | Neutral hover and inset fills |

Users can override the color tokens per theme in Settings → Display (`lib/theme-colors.ts`, `#RRGGBB` only, stored per browser), so components must read colors from these tokens to honour it.

Light mode uses exactly two neutral tones (zinc grey page, white cards) with no hue tint; structure comes from borders. Empty-state notes are `rounded-lg border border-dashed border-border`, not grey fills. Never fade text with opacity (`text-muted-foreground/30`, `text-text-tertiary/50`): use `text-text-tertiary`, the faintest token that still meets 4.5:1. Opacity is fine for decorative icons and separators.

### Borders
- **Default:** `border-border` — card borders, dividers
- **Subtle:** `border-border/50` — lighter dividers within cards, section separators
- **Interactive:** `border-hairline` idle, `border-hairline-strong` hover — for buttons, inputs, interactive chrome

### Semantic Status
- **Success/Online:** `#34d399` — active services, healthy nodes, approved grants
- **Warning/Expiring:** `#f59e0b` — expiring tokens, pending approvals, attention items
- **Error/Expired:** `#f87171` — expired tokens, failed requests, denied approvals
- **Info/Auth:** `#60a5fa` — informational badges, auth events

### Usage Rules
- Purple accent ONLY on: active navigation icon color and current conversation-index tick (`text-nyx-secondary-400`), sidebar control active dot, the `variant="primary"` button (gradient — used as the dominant action in dialogs, onboarding, and right-panel cards), AI setup card glow, role badges (`accent` variant for owner), logo
- Purple NOT on: idle surfaces, every hover state, surface tints on idle cards, decorative backgrounds. Hover stays neutral (`bg-overlay` / `bg-overlay-strong`).
- Buttons: Approve = success green background tint, Deny / destructive = error red background tint, Secondary = ghost with border, Primary CTA = `variant="primary"` (`nyx-gradient-vivid`) — used on dialog submits, onboarding CTA, and right-panel promo cards
- The ambient status line at the top of the viewport is colored by node-fleet health: success gradient when healthy, warning when draining, destructive when offline (`AmbientStatusLine`, 2px, fixed top)

## Typography
- **Display/Headings:** Mona Sans — page titles, metric values and card headings use the weights in the role table.
- **Body:** Mona Sans 400 — body text, descriptions and row content.
- **UI/Labels:** Mona Sans 500 — button labels and form labels; navigation uses 400 at rest and 500 when active.
- **Data/Tables:** JetBrains Mono 400 — timestamps, log entries, API paths, code snippets. Supports tabular-nums.
- **Logo:** Playfair Display 400 — NyxID wordmark only (used on landing/legal pages; the logged-in app shows only the icon). Letterspace: 1px. Color: `#c4b5fd`.
- **Loading:** Google Fonts CDN

### Scale (in-app)

Numeric tokens have one universal definition: `text-N = N/16rem`, on every surface,
including unthemed content. Define each `--text-N` once in `app.css`'s `@theme`;
never override token values for a surface, theme, or breakpoint. The defined
numbers are 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 22, 24, 28, and 36.

Themed Studio, NyxBot, auth, docs, legal pages and body portals share the root
scale and a unitless body line-height of 1.5. At a 16px browser default,
`text-N` renders `N × 0.85 × text multiplier` CSS pixels. New 1× has the previous
0.85× root (13.6px); `text-14` is 11.9px and `text-16` is 13.6px at 1×.
Landing/blog and standalone unthemed protocol pages retain their browser-default
root. Numeric token definitions are identical there; shared primitives use explicit
`unthemed:` assignments where necessary to preserve public presentation.

Settings → Display offers text multipliers 0.5×–2×, in 0.01× steps. Previous custom
preferences migrate once to `clamp(value / 0.85, 0.5, 2)` without rounding;
previous default users adopt the smaller root. No additional migration accompanies
token standardization. Slider labels round to two decimals. Spacing independently
scales the 4px `--spacing` unit from 0.5×–2×, with the same step. Dragging previews
the multiplier; release applies it, and native keyboard adjustments apply directly.

Use these semantic roles in both shells and all themed overlays. Shared primitive
assignments live in `lib/typography.ts` and `typography.css`. Use the `type-*`
role utilities (or `TEXT_STYLES`) in markup. Each owns size, weight and line height;
consumers customize layout and semantic color, not individual typography properties.

| Role | Token | Weight | Line height | Default CSS px (1×) |
|---|---|---:|---:|---:|
| Desktop page title | `text-28` | 700 | 1.1 | 23.8 |
| Mobile page title | `text-22` | 700 | 1.1 | 18.7 |
| Section heading | `text-20` | 600 | 1.5 | 17 |
| Card / dialog / sheet title | `text-15` | 600 | 1.5 | 12.75 |
| Navigation item, inactive / active | `text-15` | 400 / 500 | 1.5 | 12.75 |
| Body / page description / table cell | `text-14` | 400 | 1.5 | 11.9 |
| List row identity / approval service / automation label | `text-14` | 500 | 1.5 | 11.9 |
| Form label / button / action link / tab | `text-14` | 500 | 1.5 | 11.9 |
| Input / select value / menu item | `text-14` | 400 | 1.5 | 11.9 |
| Helper / secondary caption / timestamp | `text-13` | 400 | 1.5 | 11.05 |
| Field validation error | `text-13` | 500 | 1.5 | 11.05 |
| Tooltip | `text-13` | 400 | 1.5 | 11.05 |
| Group label / badge | `text-12` | 500 | 1.5 | 10.2 |
| Table column heading | `text-12` | 600 | 1.5 | 10.2 |
| Account name | `text-14` | 500 | 1.5 | 11.9 |
| Account email | `text-12` | 400 | 1.5 | 10.2 |
| Toast title | `text-15` | 500 | 1.5 | 12.75 |
| Toast description | `text-14` | 400 | 1.5 | 11.9 |
| Toast action / cancel | `text-14` | 500 | 1.5 | 11.9 |
| Transcript / composer prose | `text-16` | 400 | 1.625 | 13.6 |
| Prominent metric | `text-28` | 600 | 1.25 | 23.8 |
| Code / terminal | `text-14` | 400 | 1.625 | 11.9 |

`Label`, `FormLabel` and `GroupedLabel` share the field-label implementation:
`type-field-label` (text-14/500, 1.5 line height). Use `Label asChild` for legends
or captions whose controls already have an accessible name. Keep native `htmlFor`
or wrapping-label associations for editable fields. Never put size, weight or
line-height classes on these consumers. Inline grouped controls and stacked
editor captions use the same field-label role; help text uses the secondary role.
Use `FieldHelp` for explanatory descendants inside wrapping labels; it owns the
secondary role (or the explicit body/emphasis helper variant). The caption itself
inherits the label role. Shared `NativeSelect` owns the input role, bounded width
and native picker semantics; it never inherits a wrapping label's weight. Its
appearance reset retains the shared line box, with a decorative chevron.

Line heights are unitless role values: body, secondary, caption, label, controls
and card/dialog titles use `--line-body` (1.5); page titles use `--line-page-title`
(1.1); metrics use `--line-metric` (1.25); transcript prose, composer and code use
`--line-prose` (1.625). Plain numeric size utilities inherit the surrounding role line box.
Do not author `leading-*` or inline line-height overrides in themed consumers.
CSS consumers use role utilities through `@apply`; library renderers read
`computedTextLineHeight`. Never-themed public prose retains its own typography and 1.4 base line box; shared
primitives preserve their original public label/title line boxes with `unthemed:`.

Inline emphasis is explicit: `type-secondary-emphasis` uses text-13/500,
`type-secondary-strong` text-13/600, and `type-body-strong` text-14/600, all at 1.5.
`type-data-total` uses text-15/500 for compact numeric totals. Decorative onboarding
monograms use `type-brand-monogram` (text-14/700) and `type-brand-letter`
(text-28/700), both at 1.5; these are brand glyphs, not UI labels.
These preserve emphasis in dense data and rich content; they are not field-label,
button or title substitutes. Rich-content headings retain their hierarchy: text-24/600, text-20/600,
text-17/600, text-16/600 and text-16/500, all at the 1.5 title line box;
inline chat mentions use text-16/500 with the transcript line box. These exceptions
exist because data emphasis and nested prose headings are distinct from UI titles.
Decorative avatar initials use the shared `type-avatar-initials` roles: text-12,
text-13 for large avatars, and text-15 for thread-header avatars, each weight 600
at the body line box. Avatar counts use the corresponding group-label, secondary
emphasis or compact-data role. Their visual hierarchy follows the avatar size.
The source guard rejects new size/weight pairs, local line-height assignments,
native label wrappers and typography overrides on shared labels, including
local class constants, template fragments and important (`!`) utilities. Sonner uses important shared toast roles so vendor
styles cannot replace their weights or line boxes. Imported label aliases, inline
style assignments and nested caption overrides are checked too. Complete roles
cannot be combined with partial size assignments in JSX or CSS. JSX prop spreads
resolve inline and local object literals, including nested spreads, for both
`className` and `style`; numeric-size consumers also reject inline typography overrides. Selected shell
navigation declares its shared active role through `aria-current="page"`.
Test fixtures
and never-themed protocol/landing/blog pages are the documented source exclusions.

Workspace sidebar glyphs animate only on row activation (click/tap or Enter/Space),
never hover, pointer entry or focus. Each activation restarts one cycle from rest:
Home's chimney rises, three smoke puffs finish, and the chimney sinks; the other
icons complete their sequence once. All finish on lucide's rest frame. A brief
in-memory timeline resumes the same cycle across SPA/sidebar remounts, including
reopening the mobile drawer; navigation and drawer dismissal remain immediate.
System and Settings → Display reduced motion both keep the icons at rest.
Unavailable Workspace rows use the same `shell-nav-item` button treatment and
insets, with muted color and `aria-disabled`; focus reveals “Coming soon”.

Themed application source uses numeric roles, with `text-12` as the smallest role.
Do not use Tailwind size aliases, arbitrary font sizes, fixed CSS `font-size` or
`font:` shorthands. CSS consumers read `var(--text-N)`; SVG charts use the same
variables. Canvas consumers read `computedTextSize(N)` and refresh when Display
changes. Sonner actions/cancel explicitly set the button role to override vendor
fixed sizing. Toast content keeps its readable width; actions wrap below it and
within the toast at enlarged text settings. Inline analytics filters and chips wrap;
chart label selection uses the current scaled font measurement, refreshed after
Display changes, and skips categories when they cannot fit without overlap. The source guard runs with lint; compiled-CSS and real-component
browser tests cover both shells, portals, all five audit scales, and touch targets.

Shared controls and custom button, disclosure and summary targets reserve at least 24px,
including Studio, dialog/sheet close and body portals at the smallest Display setting.
This hit-area floor does not enlarge the visible glyph. Role assignments include both
size and weight; custom onboarding actions use the same button role as Button,
and analytics totals use the prominent-metric role.
On coarse pointers, editable inputs use `max(16px, var(--text-14))` to prevent iOS
focus zoom without changing their semantic role above that floor. Controls and
label/value rows wrap with available space; essential monetary values stay complete.
Shell navigation icons share `1.125em` sizing, labels and account footer roles.
Studio and NyxBot also share control treatment and page spacing. The themed root
owns `--display-density` and every shared control rule, including body portals;
`assistant-display` never changes shared primitive geometry. Interface icons use
`max(0.75rem, 1.125em)`. Icon/label actions use `0.5em × spacing` gaps,
`0.75em × spacing` horizontal and `0.375em × spacing` vertical padding; icon-only
controls use `0.5em × spacing` padding. Enlarged controls grow to fit their labels
with the shared 24px floor. Grouped rows use four spacing units horizontally,
2.5 vertically, and an eleven-unit minimum height. Select indicators, tab strips,
range inputs and grouped fields have one treatment regardless of shell.
Icon-only actions opt into the shared icon-button treatment. Their display rule
lives in the base layer so responsive visibility utilities remain authoritative.
Toolbars grow and wrap their action groups at enlarged text and spacing settings.

`shell-page-content` gives both shells four spacing units of content inset on
mobile, six from 640px, eight horizontally from 768px, and ten horizontally from
1024px; top inset stays six on desktop. Bottom inset reserves at least 2rem and
the safe area. `shell-page-stack` uses six spacing units between sections;
`shell-card-grid` uses four between cards. `WorkspacePageHeader` composes the
shared `PageHeader` inside its sticky-title behavior: identical title line-height,
description gap, action gap and wrapping. Shared shell navigation, group labels
and account blocks own their padding, gaps, radii and icon dimensions.
Every expanded, collapsed and mobile navigation stack uses `shell-nav-stack`,
with half a spacing unit between rows. Service cards in Studio and Plugins share
`ServiceCardSummary`, composing `ServiceCardHeader`: the card-title role with tight tracking and a ten-spacing-unit
icon container. Both keep the same icon surface, border, corners and title gap. The summary owns
two spacing units from header to description, a body-role description at 1.5 line
height, and two reserved lines (`2lh`). Plugins retain content-driven equal rows.
The header owns one visual title line and subtitle offset. An interactive title
uses a vertically centered hit target outside that line's layout, preserving the
shared control minimum without pushing the subtitle down. Approval identity and
action fields share their row-title and body roles in mobile cards, desktop tables
and NyxBot History. Trigger labels use the row-title role in both lists; machine
summary cards use the card-title role in both shells.
Toasts follow the app theme; titles use foreground and descriptions use
muted-foreground on the actual toast surface, including typed notifications.
All title, description and action text must meet 4.5:1 contrast.

The only remaining `assistant-display` CSS rule exempts conversation-index tick
slots from general control minimums. It is a transcript navigator, not a shared
control. Chat transcript/composer/slab layout remains component-scoped because
Studio has no corresponding surface. Never-themed pages retain their own defaults.
The treatment parity browser matrix mounts identical primitives in the real Studio
and NyxBot Approvals routes and measures both inline and portaled controls at
1440/390, both themes, text 0.5/1/1.5/2× and spacing 1/1.4×. Real navigation,
account metadata and equivalent page spacing are also checked.

Mobile Activity cards lead with the service name in `text-15 font-semibold`; the action uses `text-14 font-medium` and the URL uses secondary `text-13`. Service names may wrap to two lines.

## Components

All component and page recipes use the semantic roles in the Scale table,
including their weights. Numeric class snippets name those roles; the number
is the token's canonical size before the Display base and multiplier apply.

### Service pool and routing icons

Use `ServicePoolIcon` and `PoolStrategyIcon` from
`components/dashboard/service-pool-icons.tsx` for pools and their routing
strategies. Priority routing is the default visual reference: each load is a
hollow circle, connected by a rounded stroke. All variants share Lucide's 24×24
viewBox, 2-unit stroke, rounded caps and joins, and `currentColor`. Load nodes
share a 3-unit circle radius. Weighted routing uses a tilted balance beam with
circular loads at different heights; round robin arranges three loads around a
cycle with separated clockwise arrows and visible gaps before each node; the
service-pool mark is a circular ring with five evenly spaced load nodes.
Each strategy must have a distinct silhouette at the 14px inline size. Reuse this
family in cards, tables, summaries, route panels, and routing controls so the same
shapes retain their meaning.

### Buttons
- **Height:** themed controls use intrinsic height with a shared `max(24px, 1.5rem, 1lh + 2 spacing units)` floor; labels and padding determine the final height. Size variants retain their horizontal insets, and icon controls use the shared icon-only treatment. Never-themed defaults retain `h-8` / `h-7` / `h-9`.
- **Text:** `text-14 font-medium`
- **Radius:** `rounded-lg` (shared radius token)
- **Icon size:** themed controls use `max(0.75rem, 1.125em)`; never-themed defaults retain `size-3`.
- **Gap:** themed icon–label actions use `0.5em × spacing multiplier`; text-only actions retain their size variant’s insets.
- **ButtonIcon:** `h-[18px] w-[18px] rounded-[4px]` — small inset icon container for primary/destructive buttons
- **Variants:** default (ghost border), primary (gradient — used as the dominant dialog/CTA submit), destructive, outline, secondary, ghost, link
- **Loading:** `isLoading` prop shows `Loader2` spinner + children, disables button. Some legacy pages still use a label-swap (`{mutation.isPending ? "Creating..." : "Add"}`) — prefer `isLoading` for new code.
- **Rule:** Edit Save buttons are disabled while pristine, invalid or pending. Create actions are disabled only while invalid or pending; valid prefilled defaults can be submitted.
- **`AddCtaButton`** (shared) — the standard "create" trigger on every list page header. `h-8`, `rounded-lg`, `border-hairline` idle / `border-hairline-strong` hover, button-role `text-14 font-medium text-text-tertiary`, with a 22×22 inset icon container (`rounded-[6px]`, `border-hairline`, `bg-overlay`, 12px Plus icon). Distinct from `variant="primary"` — used when the button is the page-level "add" affordance.

### Composer toolbar
NyxAgent permission/model controls use a wrapping `h-7` toolbar with the
input/menu-value role (`text-14 font-normal`) inside the composer border below
its full-width textarea, with send/stop/mic on the right; composers without a
toolbar keep the existing inline layout.

### Inputs
- **Height:** intrinsic with the shared control minimum; input padding keeps the text clear of its edge
- **Text:** `text-14`
- **Padding:** `px-3 py-1.5`
- **Radius:** `rounded-lg`
- **Border:** `border-input`, focus: `border-input-focus`
- **Placeholder:** `text-text-tertiary`

### Selects
- **Trigger height:** intrinsic with the shared control minimum
- **Text:** `text-14`
- **Padding:** `px-3 py-1.5`
- **Trigger radius:** `rounded-lg`
- **Item radius:** `rounded-md`
- **Item padding:** `py-1.5 pl-3 pr-8`
- **Chevron:** `h-3.5 w-3.5`

### Badges
- **Padding:** `px-2 py-0.5`
- **Text:** `text-12 font-medium`
- **Radius:** `rounded-md`
- **Variants:** `default` (purple), `secondary` (`bg-muted`), `destructive`, `success`, `warning`, `info`, `accent` (purple, slightly softer fill)
- **Dark = tuned tint, light = solid fill (theme-split).** Dark is the primary surface: the base (unprefixed) classes are the tint `border-{color}/30 bg-{color}/{10|15} text-{color}` and are left as-is. Light mode deviates to a SOLID fill via the `light:` variant — `light:border-transparent light:bg-{color} light:text-white` — because the low-opacity tint loses contrast on a light canvas. Every color-carrying variant follows this in `components/ui/badge.tsx`. `secondary` is the neutral badge: it keeps its `bg-muted` fill in dark, but in light its fill matches the canvas and vanishes, so it deviates to an OUTLINE (`light:bg-transparent light:border-muted-foreground` — border in the same `muted-foreground` as its text). `{color}` is a semantic token (`destructive`/`success`/`warning`/`info`/`nyx-500`/`nyx-secondary-500`). The `light:`/`dark:` variants are class-scoped to `html.theme-{light,dark}` in `app.css`; never-themed public pages (landing/blog) keep the dark base. Do NOT give a pill a single *theme-blind* raw-palette class as its only color (e.g. `text-red-200 bg-red-500/10` with no variant) — the `-200`/`-300` tints wash out in light; always pair a dark tint with a `light:` solid.
- **Severity / risk pills** (`lib/constants.ts` `scopeRiskClass`): same theme-split convention; map the scale onto tokens high→`destructive`, medium→`warning`, low→`success`.
- **Role badges** (`RoleBadge`): owner → `accent`, admin → `info`, member → `success`, viewer → `secondary`. Use these mappings consistently for any role/permission UI.

### Tables
- **Head height:** `h-8`
- **Head text:** `text-12 font-semibold uppercase tracking-[1.5px] text-text-tertiary`
- **Head padding:** `px-3`
- **Cell padding:** `px-3 py-2.5`
- **Cell text:** `text-14 text-foreground`
- **Row border:** `border-b border-border`
- **Container:** wrap tables in `rounded-xl border border-border/50 bg-card overflow-hidden` for the desktop view of a list page
- **Rules:**
  - Actions column should only be present when the table has row actions (no empty "Actions" header)
  - Tables with 2+ row actions should use a 3-dot dropdown menu instead of inline buttons (a few legacy pages — `approval-history`, `admin-user-detail` — still inline; convert when touching them). Note `approval-history` already correctly hides the column when no row needs actions (`{requests.some(r => r.status === "pending") && <TableHead>Actions</TableHead>}`); the only violation there is the inline Approve/Reject buttons.
  - Tables on list pages must be paired with the [Mobile-card / desktop-table responsive split](#list-page) (see Page Patterns)

### Switches
- **Track:** `h-5 w-9`
- **Thumb:** `h-4 w-4`
- **Checked translate:** `translate-x-4`

### Tabs
- **List height:** `h-8`
- **Trigger padding:** `px-3 py-2`
- **Trigger text:** `text-14 font-medium`
- **Active state:** `text-foreground` + 2px bottom indicator line
- **Content margin:** Two equivalent patterns are in use — either `mt-6` on each `TabsContent` (e.g. `keys.tsx`, `org-detail.tsx`) or `space-y-6` on the parent `<Tabs>` wrapper (e.g. `settings.tsx`, `consents.tsx`). Both produce the same 24px gap between trigger row and content. Pick one per page; the legacy `mt-3` only survives in primitive demos.
- **Indicator:** animated sliding `bg-primary` bar

### Dialogs
- **Padding:** `p-5`
- **Gap:** `gap-4`
- **Radius:** `rounded-xl`
- **Title:** `text-15 font-semibold`
- **Description:** `text-14 text-muted-foreground`
- **Close button:** top-right, shared minimum 24px target with an inset X icon
- **Submit button:** `variant="primary"` is the default treatment for the affirmative action; Cancel is `variant="outline"` or `variant="ghost"` to its left

### Dropdown Menus
- **Content radius:** `rounded-xl`
- **Content padding:** `p-2`
- **Item padding:** `px-3 py-1.5`
- **Item text:** `text-14`
- **Item radius:** `rounded-md`
- **Item gap:** `gap-2`
- **Hover:** `bg-overlay-strong`

### Checkboxes
- **Size:** `h-4 w-4`
- **Radius:** `rounded-[4px]`
- **Check icon:** `h-3 w-3`
- **Checked:** `bg-primary border-primary`

### Popovers
- **Radius:** `rounded-[12px]`
- **Padding:** `p-4`
- **Shadow:** `shadow-lg shadow-primary/5`

### Tooltips
- **Radius:** `rounded-[6px]`
- **Padding:** `px-3 py-1.5`
- **Text:** `text-13 font-normal`

### Grouped lists (assistant forms)

NyxBot forms use iOS inset grouped lists from
`components/ui/grouped-list.tsx`. Stack sibling groups with `space-y-6` on the
`bg-background` canvas. Each group has an optional sentence-case heading in the
group-label role (`text-12 font-medium text-text-tertiary`) above, one
`rounded-xl bg-card border border-border/50` surface, and an optional footer in
the helper role (`text-13 font-normal text-text-tertiary`) below. The subtle
border provides contrast in light mode and also works with
custom theme tokens. Headings and footers use `px-4`.

`GroupedRow` provides a `min-h-11` touch target, `px-4 py-2.5`, wrapping titles
and accessories, and a hairline before adjacent rows inset by `left-4` (or `left-11` after a leading icon).
Optional leading icons use `1.125em`. Use `asChild` to retain the native
button, link, label or form-item element. Only interactive rows receive hover and pressed styles. Keyboard focus uses an inset
`input-focus` ring on the whole row, with the card’s corner radius on first and last
rows; focus within adds a subtle `bg-overlay`. Text-field rows use only this background and the caret, with no field or row ring. Action and disclosure rows keep the inset ring. `GroupedDisclosure` is a native button with
a trailing chevron; buttons and links must keep accessible names.

Groups never contain another group, card or outlined control. Use borderless
`GroupedInput`, `GroupedTextarea` and `GroupedSelectTrigger` inside rows.
Short fields can pair a leading label and trailing value; longer fields grow
under their label. Account text fields consistently stack a label above a
left-aligned, full-width input, including Name, Email and password fields.
Editable names have a pencil affordance; empty password fields have explicit
entry placeholders with visibility controls inside the row. Email keeps its
verification badge beside the label and truncates long read-only values,
retaining the full address in its title. Range rows use the shared
`RangeControl`: label and current value aligned above a full-width slider,
with one muted helper below and a reset action beside the value. Native range
semantics expose the bounds; do not stack min/max and default captions. Validation text is destructive and linked with
`aria-describedby` (the shared Form primitives provide these links). Put
field-specific explanations directly below their field in `text-13 text-text-tertiary`; reserve the group footer for shared context, warnings and errors. Never concatenate unrelated field help in a footer. Destructive
actions are destructive-colored rows in their own neutral group.

Single on/off settings and acknowledgments use Switch; Checkbox is reserved for
multi-select lists. Selection lists use checkmark or switch rows, search when there are more than
eight items, and scroll internally only for long lists. `GroupedFormScope viewport` gives
the whole drawer or inline settings surface one dirty-only `GroupedFormBar viewport`. Use a compact row with `py-2`, right-aligned Discard and Save changes; wrap actions to another row when needed and allow long button labels to wrap within their insets. Announce unsaved section names through sr-only live text in the “Unsaved changes” region, with no visible unsaved label. Put this bar outside the scrolling `FormViewport` as a flex sibling, pinned to the viewport bottom. It remains available during search and detail navigation. Mobile sheets track `visualViewport` resize/offset changes so the actions stay above the keyboard; menus use the editing viewport as their collision boundary and scroll their own content above the actions. Save runs each
dirty form in order with its own validation, payload and revision fence. A failed
form keeps its draft and local error; successful forms stay saved. Discard resets
all dirty forms in that scope. Save uses `variant="primary"`; Discard uses ghost
or outline. The pinned bar has an opaque `bg-background` and a top hairline
`border-border/50` so scrolled footer text cannot show through. Each save pass
reads current registrations and attempts each eligible form at most once; a form
that becomes dirty while an earlier request awaits is included in that pass.
Removed forms are skipped. Changes after a form's attempt remain dirty for the
next pass. Closing the drawer or leaving a detail page with its own dirty forms
asks “Discard unsaved changes?” with Keep editing and Discard actions. Drawer
closure checks all tabs; detail Back checks only that page and its descendants.
Drill-ins share the drawer scope and preserve each
operation editor's revision fence. Inner bars are suppressed when a viewport bar owns the forms. Immediate controls remain immediate only where their
existing behavior requires it.

`grouped-fields.tsx` retains the shared Form field IDs while placing labels,
inputs and linked errors in row and footer slots. `GroupedDialog`, `GroupedSheet`
and `GroupedPage` register dirty drafts for X, Escape, overlay, route navigation
and browser unload. Each dialog isolates its save scope from a settings form
behind it. Immediate reviewed decisions retain their existing buttons and register
only their acknowledgment draft. Dialogs always show Cancel (ghost/outline) and a
primary action through `GroupedDialogActions` (or `GroupedFormBar dialog` for a
coordinated edit). They never show Unsaved or Discard. Edit Save additionally requires
a dirty form; Create can submit valid pristine defaults. Cancel follows the same
discard guard as X, Escape and the overlay. Dirty-only bars are for inline drawer
tabs/drill-ins, the Account Settings and NyxBot settings panels and machine
settings sheets. Account Settings uses the same background for its header and
body, segmented tabs, and one save bar outside the tab scroll viewports.
Visited tabs remain mounted to preserve drafts, and tab changes retain them
without a discard prompt. Studio `/settings` shares these grouped sections
and reviewed saves; its page header and route remain intact. Display and
privacy controls retain their immediate persistence. Persisted
request retries stay explicit actions, available without a new draft;
they preserve the existing acceptance, idempotency and revision checks.
Light-theme destructive text uses the darker semantic red so field errors in
footers meet 4.5:1 contrast against the page background as well as cards.

Every edit Save first opens `Review changes`. Register a `changes()` describer
with each form: compare its defaults with current values using an explicit
allowlist of fields. Section headings use sentence-case display labels. Show human
select labels and On/Off booleans in one line: “Field: before → after”, with the replaced value tertiary and struck through, then the foreground new value. Truncate with ellipsis, retain the full line in its title and offer Show more when it overflows. Lists use separate single-line + added / − removed rows with the same expansion behavior. Never
enumerate arbitrary form values or expose secret/token fields; sensitive
destinations are described as changed without showing their contents. Keep
editing returns to the draft. Save changes runs the existing callbacks and
fences, reports successful section names in a toast, and retains failed drafts
and local errors. Create dialogs keep their direct Create action.

`components/ui/segmented-control.tsx` wraps Radix Tabs: a `bg-muted` track,
raised `bg-card shadow-sm` selected segment, `text-14` labels, arrow-key
navigation and associated tab panels. Segments size to their labels and wrap
onto another line when the shared Display scale needs more space. Keep inactive form panels mounted and
hidden so switching tabs or returning from a detail view preserves drafts.
Drawer deep links focus the selected panel heading; detail Back restores focus
to its disclosure. Keep the drawer full width on mobile and `sm:max-w-lg` on
desktop. All spacing, text and icons follow the shared Display scale.

The conversation index floats at the chat column’s outer right edge next to
the transcript, before its scrollbar and any preview, CMA or machine column,
with no panel fill or hairline. Its containing viewport belongs to the chat
column, so sidebar visibility and saved preview widths move it with the chat.
Below 768px the rail is hidden and reserves no space; touch readers navigate
with the transcript and jump-to-latest control. Right-aligned, round-capped ticks use a 7px uniform pitch
at 1× text, 16px long assistant ticks and 8px short user ticks. Thickness is
1px on retina displays, increased to 1.5px at 1dppx for crispness. Geometry
scales with text independently of spacing. This dense graphical navigator
explicitly opts out of text-button minimum dimensions. Resting decorative
ticks use `text-text-tertiary` at 50% opacity; their accessible message labels
remain fully available. Hover, press and keyboard focus use full-opacity
`text-muted-foreground`, and only the current tick uses full-opacity
`text-nyx-secondary-400`. Selection changes color only: assistant ticks remain
16px and user ticks remain 8px, including during focus and dragging. Tick lengths and thickness scale with
text independently of pitch.

The rail’s scroll viewport fills the available transcript height below the
toolbar and above the composer or bottom inset. Measure these visible bounds
in the chat column; never subtract the composer reservation twice. Short tick
sequences stay centered at the same 7px pitch, and longer sequences overflow
inside the rail without stretching or compressing their slots. Keep tick
rendering bounded to that viewport and its overscan. Reuse the
transcript's edge-aware mask pattern: fade only an edge with more ticks beyond
it, and remove that fade at the revealed edge. Wheel browsing, dragging (with
edge autoscroll), and keyboard navigation scroll the rail; transcript navigation
keeps the current tick visible. Rail scrolling is immediate, including with
reduced motion. A passing pointer shows no preview. Pressing or dragging a tick
previews its message until release; keyboard focus also previews. Click, drag
and keyboard navigation jump to messages. Observe the toolbar size to keep the rail below
its controls, including wrapped compact headers. Defer rail position and size
writes to an animation frame outside ResizeObserver delivery: moving the
rail's top also changes the measured viewport height during header animation.

### Agent drawer search

The agent details drawer has matching circular Search and Close header icon
buttons, using eight spacing units (about 32px), `bg-overlay`, a stronger neutral
hover fill and a visible keyboard focus ring. Search is named “Search {agent title}
agent details” and exposes `aria-expanded`; Close is named “Close agent details”
and retains Sheet dismissal semantics. Search expands into a compact inline field
between the agent name and Close, with autofocus, a leading icon, a clear action
and the same accessible name/placeholder. Below a 480px drawer container width,
open search hides the avatar/name with a short transition and fills their header
row. The search glyph remains leading at every width; do not add a back arrow. Measure the placeholder against
the available input width: shorten it to “Search {agent title}” when needed, then
“Search agent” for unusually long names; its accessible name always stays full.
Empty blur collapses it to its icon;
a nonempty query keeps it open. Search and tabs use compact spacing (`text-14`, minimum 24px targets)
and scale with the assistant reading setting. Nonempty searches replace the
visible panels with inset result groups named for their tabs. Results are native
buttons with a title, a breadcrumb subtitle and emphasized matching text; a polite
live region announces the result count. No matches reads `No results for “…”`.

Sections explicitly opt into the typed `drawer-search` registry from their own
live render data. Stable IDs identify entries and reveal targets. Index only
entries shown for the current agent, permissions and flags; no DOM-derived index
or separate inventory. Match case and diacritic insensitively, requiring every
query word; rank exact titles, title prefixes, title substrings, then metadata.
Cap results at 80 and group them in tab order. Channel chat metadata is fetched
with its existing GET hook while the drawer is mounted so unopened chats are
searchable; chat controls and mutations keep their existing behavior.

Search retains mounted tabs and visited detail pages, including their drafts.
While searching, one opaque save bar aggregates dirty forms across the drawer,
using each form’s existing validation, payload, fence and local error. Revealing a
result uses the existing detail discard guards, switches tabs, opens its detail
page if needed, then scrolls and focuses the target row and briefly highlights it.
The highlight has no motion when reduced motion is requested. Detail pages occupy
the content region below search; covered tabs and controls remain inert.
Back retains the usual detail navigation. Focus restoration never takes focus
away from someone already typing in search.

Inside the drawer, `/` and Cmd/Ctrl+F focus search unless an editable control owns
focus. Arrow keys move through results and Enter reveals the selected result
(or the first result from the search field). The first Escape clears a nonempty
query; Escape on an empty open field collapses it to the Search button; the next
Escape follows normal sheet dismissal and dirty-draft guards.

Every drawer group heading and drill-in title includes a small `SectionInfo`
button through its optional `info` prop. Explicit plain-language copy explains
the section in one or two sentences and joins its search keywords. The named
“About {section}” button opens the same controlled help surface on hover,
keyboard focus or touch, supports Escape, and keeps content within about 280px.
Required warnings and instructions remain visible in the form.

Drawer and settings headers use `CompactTitleProvider`, `CompactChrome` and
`CompactScroll`. IntersectionObserver watches separate collapse and expand
sentinels (24 and 8 spacing-adjusted pixels) for hysteresis; measured chrome
height is replaced with an equal scroll inset so content does not jump. Transform
and opacity transitions ease over 240ms, including a measured translate animation
for the segmented control as the chrome changes height. No per-frame scroll
handlers or layout animations. At the top the drawer shows its full identity and
subtitle, with circular Search and Close aligned to the name, followed by compact
tabs. When collapsed it has only two rows: small avatar/name and the same header
controls, then compact tabs. Keep the same input mounted and focused across both
states; opening search reveals it inline with a 200ms clip/opacity transition.
The Close button stays fixed in place. Reduced motion switches instantly.

### Assistant workspace pages

Approvals, Plugins, Automations and Machines use `WorkspacePageHeader` inside the
workspace's common scroll and padding container. Titles, description width and
wrapping action slots share one scale. Automations and Approvals headers and content
share `max-w-2xl`; Plugins keeps its wide grid. Approvals use Pending/History segments and
local-day history groups; the workspace wrapper adapts the existing approval
block without changing its decision handling or transcript presentation.

On desktop, `StickyTitle` observes the large heading inside its scroll owner.
Once that heading leaves view, an opaque compact title and primary-action bar
appears without taking layout space. Keep one accessible heading; the compact
title is a presentation mirror. The original action container becomes inert
while the mirror action is available, avoiding duplicate tab stops. Mobile
retains the page header. Reduced motion disables the short fade/slide.

Plugin catalog cards share `PluginCard`, including skill search results. Grid rows
size equally to the largest card's content, with a minimum of five rem plus
eighteen spacing units. Height grows with content and independent text/spacing
settings; never fix card height or let actions escape into the next row.
Names and status lines truncate with native titles, descriptions reserve two
lines, and the footer stays at the bottom. The primary native button/link covers
the card; secondary controls are siblings inside the footer. Neutral overlay
hover and an outer focus ring keep the full card actionable without nested
buttons. Attached skills remain inset grouped rows. Platform-bound and automatic
connections stay hidden; personal model setup is one Add a model connection card opening the provider picker.
Workspace segments are capped at `max-w-80`; search and approval content share
`max-w-2xl`, while catalog grids use the available page width.

### Sheet (slide-over panel)
- Used for inline detail/edit on list pages where a full route is overkill (e.g. `admin-invite-codes` row detail). `sm:max-w-lg`, otherwise inherits Dialog tokens. Prefer Sheet over Dialog when the panel contains substantial reading content alongside actions.

## Spacing
- **Base unit:** 4px
- **Density:** Compact
- **Scale:** 2xs(2px) xs(4px) sm(8px) md(16px) lg(24px) xl(32px) 2xl(48px)
- **Content padding (main area):** `px-4 pt-4 sm:px-6 sm:pt-6 md:px-8 lg:px-10`, with `paddingBottom: max(2rem, var(--sab))` for safe-area-aware bottom space
- **Card padding:** `p-4` (standard), `p-3` (compact/quick links)
- **Nav item padding:** `py-2`, expanded: `gap-3 px-3`, collapsed: `justify-center px-0`
- **Gap between sections:** `gap-8` (dashboard), `gap-4` (card groups), `space-y-8` (stacked `DetailSection`s on detail pages)
- **Gap between cards:** `gap-3` (right panel, dashboard status grid), `gap-4` (card groups)
- **Gap within cards:** `gap-3` (title to content), `gap-2.5` (between rows)

## Layout

### Shell Structure
Top bar spans full width. Sidebar + content sit below it. The right panel is **opt-in per page** via the `RightPanelContext` (a page calls `setRightPanel(...)` to register content for the slot).
```
+--------------------------------------------------+
|  [bar]  ← AmbientStatusLine (2px, fixed top)     |
+--------------------------------------------------+
|  [logo]   [breadcrumbs]      [search][user][gh]  |  <- TopBar (52px, full width)
+--------+-----------------------------------------+
|        |                                         |
| Side   |  Main Content         | Right Panel     |
| bar    |                       | (280px, opt-in) |
|        |                       |                 |
+--------+-----------------------------------------+
```

### Top Bar
- **Height:** `h-[52px]`
- **Border:** `border-b border-border/60`
- **Logo zone (desktop):** width tracks the sidebar (`var(--sidebar-width)`), 16px left padding, NyxID icon (`h-5 w-5`), links to `/dashboard`
- **Logo zone (mobile):** when not on a root page, replaced with a back arrow (`window.history.back()`); otherwise the icon links to `/dashboard`
- **Breadcrumbs:** rendered inline left of the actions on `md+`. `text-14`; intermediate crumbs are links (`text-text-tertiary` → `text-foreground` on hover), the last crumb is plain `text-muted-foreground` with `aria-current="page"`. `lib/studio-breadcrumbs.ts` resolves each Studio route and its URL-selected subsection. Parent links explicitly select the appropriate `tab` or `view` and clear unrelated action/callback parameters. Detail pages register their entity name via `useBreadcrumbLabel(label)`; parent entity names can be registered with an explicit path. Permission-dependent tabs publish the visible section via `useBreadcrumbSection(tab)`.
- **Actions (right):** Search trigger (opens command palette, shows `/` keybind hint), profile dropdown (icon button → menu with name/email/Settings/Log out), GitHub link. All actions use the same `border-hairline` chrome.
- **Mobile:** profile dropdown + hamburger (`Menu`) that opens the full-screen `MobileNav`

### Command Palette
Triggered by the search button or `/` key. Searches all sidebar destinations plus a small "actions" group ("Connect a Service", "Create API key", "Review approvals"). Items live in the shared `ALL_ITEMS` list in `command-palette.tsx`.

### Sidebar
- **3 modes:** Expanded (200px), Collapsed (52px), Expand on hover (52px in flow + 200px overlay)
- **Mode persistence:** `localStorage` key `nyxid:sidebar-mode`. The current width is also written to `--sidebar-width` so the top-bar logo zone can match it.
- **No logo** — logo lives in the top bar
- **Section labels** (when expanded): `text-12 font-medium uppercase tracking-[1.5px] text-text-tertiary/50`, prefixed with horizontal padding `px-3 my-2`. Labels: "Approvals", "Developer", "Admin". The "Main" group has no label (it leads).
- **Collapsed mode:** label is replaced with a short divider line (`mx-auto w-3 border-t border-border/40`) so the visual separation survives.
- **Active nav item:** `bg-overlay-strong font-medium text-foreground`, icon: `text-nyx-secondary-400`
- **Hover:** `bg-overlay`
- **Shared shell typography:** Studio and NyxBot use `shell-nav-item` (`text-15`, normal at rest / medium when active), `shell-group-label` (`text-12 font-medium uppercase tracking-[1.5px] text-text-tertiary`), and `shell-user-name` / `shell-user-email` (`text-14 font-medium` / `text-12`). User blocks use those same roles in mobile navigation and portaled account menus.
- **Nav item:** icons in both shells use `max(0.75rem, 1.125em)` relative to the shared nav label, `py-2` height, `gap-3 px-3` when expanded / `justify-center px-0 gap-0` when collapsed. Labels use the remaining row width and truncate only when their text exceeds it. Collapse transitions via `max-width: 0 → 100%` and `opacity: 0 → 1` (animated, never conditionally rendered, so layout never jumps).
- **Sidebar control:** bottom popover with 3 mode options, active option indicated by purple-filled `Circle` dot
- **Expand on hover:** outer `<aside>` is 52px in document flow, inner `<div>` is absolutely positioned and transitions width with shadow on the expanded state. 120ms enter delay, 250ms leave delay (via refs) to prevent flicker. Content is never blocked.

### Mobile Nav
Full-screen takeover (`fixed inset-0 z-[80]`) with `slide-in-from-bottom` enter / `slide-out-to-bottom` exit. Header with NyxID icon + wordmark + close button. Search input (`h-10 rounded-xl`) at top — typing filters the same `ALL_ITEMS` list as the command palette. Sections mirror the sidebar (Main → Approvals → Developer → Admin) with shared `shell-nav-item` (`text-15`) items in `rounded-xl px-4 py-3` rows; active state is a highlighted full-width row (`bg-overlay-strong font-medium text-foreground`), not a fixed-size square. Footer: a user-info row (`h-8 w-8` icon tile holding a Lucide `User` glyph — not an avatar image — plus name and email) above two full-width split buttons (Settings / Log out, both `rounded-xl py-2.5`), safe-area-aware bottom padding.

### Right Panel
- **Width:** `w-[280px]`
- **Visibility:** `hidden lg:flex shrink-0` — but the `<aside>` is also **conditionally rendered**: it's only mounted when a page has registered content via `setRightPanel(...)`. Pages without right-panel content have no aside element in the DOM.
- **Padding:** `px-3 pt-6 pb-6`
- **Card gap:** `gap-3`
- **Wiring:** A page opts in via `useRightPanel().setRightPanel(<...>)` inside `useEffect`, returning `setRightPanel(null)` on cleanup. Pages that have right-panel content also render it inline below the main content on `lg-` so mobile/tablet still see it.

### Bespoke escape-hatch layouts
A small number of routes intentionally render outside `DashboardLayout` and own their entire viewport. The current example is `/ssh/$serviceId/terminal` (`ssh-terminal.tsx`): `flex h-dvh flex-col bg-[#0f172a]` with a slim 8px-padded top bar (back button + service title + disconnect) and a terminal that fills the remaining height. No sidebar, no top bar, no right panel. Use this pattern only when the screen is a single immersive tool (terminal, fullscreen viewer) where sidebar/breadcrumb chrome would steal the whole point. Never reach for it just to "make a page feel bigger" — the dashboard chrome is the default.

### Border Radius
- **Cards, panels, dialogs, banners, code blocks (large):** `rounded-xl` (12px)
- **Buttons, inputs, nav items, code blocks (small/inline):** `rounded-lg` (8px)
- **Dropdown items, select items, badges:** `rounded-md` (6px)
- **Tooltips:** `rounded-[6px]`
- **`ButtonIcon` inset:** `rounded-[4px]`

## Navigation Structure
Sidebar organized into 3 groups (4 with admin) separated by labeled section headers:

**Main** — Dashboard, AI Services, Organizations, Nodes, Channel Bots, Settings, Access & Auth, Guide

**Approvals** — Notifications, Approval History, Active Grants

**Developer** — Developer Apps, AI Setup, Integration

**Admin** (visible only to users with admin or operator role; admin pages share the same dashboard chrome — there is no separate admin layout. Operators see admin pages read-only.) — Users, Invite Codes, Audit Log, Service Accounts, Roles, Groups, Node Registry, Services, Providers

The assistant shell has its own workspace navigation and a shared account menu
in the sidebar user row and header profile button. Settings, Billing & Usage and
NyxBot settings open as URL-driven modals (`?panel=settings|billing|nyxbot`) over
the current assistant view, never as in-shell pages. Namespaced `panelTab` and
billing filter keys preserve the underlying view's search. NyxBot settings links
target these panel URLs; the header gear opens the same NyxBot panel. Billing is
capability-gated and NyxBot settings requires the NyxAgent engine. Notification
settings links to Studio at `/approvals/settings` with a visible Studio hint.
Open Studio and Log out complete both menus. Studio `/settings` and `/billing`
retain their pages and chrome. Lago checkout returns to Studio `/billing`.
Panels push on open and explicit close (Back after close reopens); Settings tabs
replace, while Billing user changes push and automatic cleanup replaces.

### Naming reconciliation
The sidebar label and page title can drift; track this when writing breadcrumbs or copy:

| Sidebar | Page title (`PageHeader`) | Tabs (if any) |
|---------|---------------------------|---------------|
| AI Services | Services & Credentials (`keys.tsx`) | External Services, Service Pools, Agent Keys |
| Access & Auth | Access & Authorizations (`consents.tsx`) | Authorized Apps, Authorizations |
| Settings | Account Settings (`settings.tsx`) | Profile, Security, Sessions, MCP, Display, Privacy |
| Nodes | Credential Nodes (`nodes.tsx`) | — |
| Guide | Setup Guide (`guide.tsx`) | — |
| Integration | Integration & SDK Guide (`integration-guide.tsx`) | — |
| AI Setup | AI Setup Guide (`ai-setup.tsx`) | — |

The breadcrumb labels in `lib/studio-breadcrumbs.ts` and the command-palette entries in `command-palette.tsx` (`ALL_ITEMS`) should match the page-title column above, not the sidebar column.

## Motion
- **Approach:** Minimal-functional
- **Easing:** enter(`ease-out`) exit(`ease-in`) move(`ease-in-out`)
- **Duration:** micro(50-100ms) short(150-200ms) medium(200-300ms)
- **Where used:** nav hover/active transitions (`duration-200`–`300`), dropdown open/close, sidebar width transition (`transition-[width] duration-200/300`), skeleton loading, tab indicator sliding (`duration-300`), card hover glow, mobile nav slide-in/out
- **Sidebar hover:** 120ms enter delay, 250ms leave delay (via refs, prevents flicker)

---

## Page Patterns

The shell tokens above are necessary but not sufficient — almost every page in the logged-in app is built from a small set of higher-order patterns. Use these primitives, not bespoke layouts.

### `PageHeader` (`components/shared/page-header.tsx`)
Every content page starts with `<PageHeader title description? actions? leading? />`.

- Layout: flex column on mobile, `sm:flex-row sm:items-start sm:justify-between sm:gap-4`
- Title: the responsive page-title role (`type-compact-page-title sm:type-page-title`), inline `letterSpacing: -0.03em`. The mobile page-title role is intentional — never override.
- `leading` slot: 32–48px avatar/icon to the left of the title (`OrgAvatar`, status dot, color chip)
- `description`: `text-14 text-muted-foreground` directly below the title group
- `actions` slot: right-aligned, `flex flex-wrap items-center gap-2 shrink-0`. Most pages put their `AddCtaButton` here; filter pages put a `Select` here; multi-action pages may stack a couple of icon buttons.
- A few legacy pages (`provider-list`, `developer-apps`, `guide`, `ai-setup`) still hand-roll a header. They typically also miss the `text-22 sm:text-28` responsive downshift (e.g. `provider-list.tsx:129` and `ai-setup.tsx:266` both hardcode `text-28`), so on mobile their titles overflow. New code uses `PageHeader`; when touching a legacy page, migrate it.

### List page
The default shape for any "list of N things" page (`keys`, `nodes`, `channel-bots`, `orgs`, `developer-apps`, all admin lists, approval lists, sessions inside Settings):

```
PageHeader (title, optional filter Select in actions, AddCtaButton)
  ↓
[optional toolbar: search input + Search/Clear buttons in flex row]
  ↓
{loading ? skeleton list :
 error   ? <ErrorBanner /> :
 empty   ? <EmptyState /> :
 mobile  ? <stack of mobile cards>     (md:hidden)
 desktop ? <Table inside rounded-xl card> (hidden md:block)
}
  ↓
[optional Pagination row]
```

- **Responsive split:** `flex flex-col gap-3 md:hidden` mobile cards + `hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden` desktop table. Both views render the same data — never show one without the other.
- **Mobile card anatomy:** `rounded-xl border border-border/50 bg-card p-4`, primary text `text-15 font-semibold`, metadata rows `text-13`, single trailing action button (`h-7 w-7` or `h-8 w-8` ghost icon) absolutely positioned top-right; multiple actions go in a 3-dot dropdown.
- A few list pages (`service-list`, `provider-list`, `orgs`) use a **card grid** instead of a table: `grid gap-4 sm:grid-cols-2 lg:grid-cols-3`. Use a card grid when each item is more "tile-like" than "row-like" (has a distinguishing icon and ≤3 fields).
- **Multi-list pages:** some pages stack more than one list on a single route (e.g. `channel-bots.tsx` shows the bots list and the device-channels list one above the other). Each list keeps its own list-page recipe (`text-20 font-semibold` section heading → toolbar → responsive split). Don't tab-ify or accordion these unless the page would otherwise scroll past two screens.

### Detail page
The default shape for any "one thing in detail" page (`key-detail`, `api-key-detail`, `node-detail`, `service-detail`, `provider-detail`, `channel-bot-detail`, `org-detail`, `admin-user-detail`):

```
PageHeader (title = entity name, leading = avatar/status, actions = Edit/Delete/etc.)
  ↓
[optional status banner: pending_webhook amber banner, org-shared green banner, etc.]
  ↓
<DetailSection title="…">
  <DetailRow label value [copyable] [badge] />
  <DetailRow ... />
</DetailSection>

<DetailSection title="…"> ... </DetailSection>
  ↓
[optional bespoke section content in its own DetailSection wrapper]
```

- Stack `DetailSection`s with `space-y-8`. Use `<Separator />` only between conceptually unrelated section groups (rare).
- For pages with many short sections, you may also use a `grid gap-4 md:grid-cols-2` of `Card`s, with full-width sections spanning `md:col-span-2`. `key-detail` and `api-key-detail` follow this; new detail pages should prefer the stacked `DetailSection` pattern unless density really demands two columns.
- Tab-based detail pages (`org-detail`) live inside a single `<Tabs>` directly under `PageHeader`. Role-gated tabs render an inline "Only admins can manage X" Card as their content rather than being hidden — preserves the tab's existence for muscle memory.

### `DetailSection` + `DetailRow`
- `DetailSection`: `rounded-xl border border-border/50 bg-card overflow-hidden` with a header strip (`border-b border-border/50 px-4 py-2.5`, title `text-15 font-semibold`) and `divide-y divide-border/30` body.
- `DetailRow`: `flex items-center justify-between px-4 py-2.5 text-14`. Left = label in `text-muted-foreground`. Right = value (`font-medium text-foreground` by default, or a `Badge` if `badge` prop set), optionally followed by a `h-6 w-6` ghost copy button when `copyable`.
- For inline editable fields (display value + pencil → input + check/X), use the local pattern from `key-detail.tsx` (don't reinvent).
- For embedded forms inside a `DetailSection`, pad the form area with `p-5` and right-align the submit button (`flex justify-end`).

### Empty state
```
flex flex-col items-center justify-center gap-1 py-12 text-center
  ↳ <Illustration className="h-64 w-64 text-muted-foreground/30" />   (or h-48 w-48 inside a section)
  ↳ <p className="text-14 font-medium text-muted-foreground/30">Headline</p>
  ↳ <p className="text-13 text-muted-foreground/30">Optional supporting line.</p>
  ↳ <AddCtaButton ... />   (optional)
```

Illustrations live in `components/icons/empty-state/` — pick a thematically relevant one (mystery box, magic key, dish antenna, robot…). Never use a generic Lucide icon at this size.

### Error state
- Inline retryable errors: `<ErrorBanner message onRetry?>` from `components/shared/error-banner.tsx`. `rounded-xl border border-destructive/15 bg-destructive/[0.04] px-4 py-3`, 36×36 icon tile, message in `text-14 text-destructive`, optional `Retry` button on the right.
- Page-level fetch failure: same shape as the empty state, but with an error-flavored headline ("Failed to load X") and the same illustration tone.

### Code blocks
Two flavors, used everywhere docs/credentials/configs appear (`guide`, `ai-setup`, `integration-guide`, `key-detail`, `nodes`):

- **Block:** `<pre className="rounded-lg border border-border bg-muted px-4 py-3 pr-12 font-mono type-code">…`. Optional filename `Badge` above. Copy button absolutely positioned top-right (`absolute top-2 right-2 h-7 w-7` or `h-8 w-8`, ghost icon button, swaps to a green `Check` for ~2s after copy).
- **Inline chip:** `<code className="rounded bg-muted px-1.5 py-0.5 text-14">snippet</code>` for short identifiers in prose.
- **`CopyableField`** (shared) — single-line value with a copy button. `rounded-xl border border-border bg-muted font-mono` body + absolutely-positioned `h-8 w-8` (or `h-7 w-7` `size="sm"`) copy button at right-center. Use for tokens, IDs, URIs displayed once.

### Settings rows
For toggle/select/input rows inside a settings card: `flex items-center justify-between rounded-lg border border-border p-4`, label + `text-14 text-muted-foreground` description column on the left, control on the right. Used in `notification-settings`, the form-row pattern in MFA/email settings, etc.

### Banners & callouts
- **Info / "important context"**: `rounded-xl border border-{color}/15 bg-{color}/[0.04] px-4 py-3` with a 36×36 `bg-{color}/10` icon tile and message text in `text-14 text-{color}` (or muted). Color picks: `success` (org-shared credential), `info`/`primary` (tip), `warning`/amber (pending action), `destructive` (error).
- **Pending-webhook / blocked state** (large, page-spanning): `rounded-xl border border-amber-500/30 bg-amber-500/10 p-6 text-center` with a centered illustration. Use when the user must take action elsewhere before the page becomes useful.
- **Empty/info microblock** (inline inside a card): `rounded-lg bg-overlay px-4 py-3 text-14 text-muted-foreground`.

### Danger zone
Bottom of any settings tab that has a destructive action: a separate `Card` with `border-destructive/40`, `CardTitle className="text-destructive"`, description in `text-destructive/70`, action button right-aligned. Always visually segregated from the regular settings form above (don't mix destructive actions into the main form).

### One-time secret reveal
For client secrets, service-account credentials, and node registration tokens, use the `ClientSecretDialog` shape:
- Dialog title: "Save … Secret" / "New …"
- `DialogDescription`: "This … is shown only once. Copy and store it securely now."
- Each value rendered as a mono code block (`rounded-lg border border-border bg-muted px-3 py-2 font-mono text-14`) with an inline `h-7 w-7` copy button
- Dismiss button: `variant="primary"` labeled "I have saved it" (not "Close" / "Done")

### Search bar
Used on every searchable list (admin lists, sessions, etc.):
```
flex items-center gap-2
  ↳ relative wrapper:
      Input  className="h-8 pl-9 text-14"
      Search icon: absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-text-tertiary
  ↳ Button variant="outline">Search</Button>
  ↳ Button variant="ghost" if query>Clear</Button>
```

### Pagination
Used at the bottom of any paginated list. `flex items-center justify-between` row:
- Left: count text in `text-13 text-text-tertiary` ("Showing 1–25 of 137")
- Right: prev / "Page X of Y" / next group, prev/next as `h-8 w-8` outline icon buttons with `ChevronLeft`/`ChevronRight`

### Stat cards / mini-grid
For a row of small KPIs inside a card or section: `grid gap-3 grid-cols-2 sm:grid-cols-4` of `rounded-xl border border-border/50 bg-overlay p-4 text-center` tiles. Value is the dominant element (use the prominent-metric role, `text-28 font-semibold`), label below in the secondary-caption role (`text-13 font-normal`). The standalone `StatCard` used on `developer-apps` is a related but slightly different pattern that should be reconciled.

### Ambient status line
A 2px gradient bar pinned to the very top of the viewport (`fixed top-0 left-0 right-0 z-[60]`), driven by `useNodes()`. Healthy = success-green gradient; draining = amber; offline nodes present = destructive red, animated pulse. Defaults to **healthy green** when the user has no nodes at all — the bar is always present and never empty. Every page sees it; pages should not draw their own equivalent.

### Onboarding (first-run + dashboard checklist)
- **Takeover:** while `useShouldShowOnboarding()` reports the user hasn't completed AI-services onboarding, `DashboardLayout` renders `OnboardingTakeover` *in place of* the normal chrome (no separate route). Centered `max-w-md` column at `pt-[12vh]`, brand mark in a `rounded-2xl` purple-tinted tile, responsive page-title role for the welcome heading, body role for supporting copy, primary CTA, Skip link below. Both actions stamp the server-side flag before unmounting.
- **Checklist:** when activeKeys === 0, the dashboard renders `OnboardingChecklist` ("Getting started", N of 3 complete) above the greeting. Mobile = vertical timeline with connector line between icon tiles; desktop = three horizontal cards with a connector line through the icon row. Done states: green check + completed copy; in-progress: purple `nyx-secondary-400` icon + colored connector. Dismissible via a top-right X (persisted to `nyxid:onboarding-dismissed`).

---

## Dashboard Content Strategy
The dashboard (after onboarding completes) follows this composition top-to-bottom:

1. **Optional `OnboardingChecklist`** — only when no API keys exist; see Onboarding above.
2. **Greeting** — `type-compact-page-title sm:type-page-title` inline `letterSpacing: -0.03em`, "Welcome back, {name}". Email below at `text-14 text-muted-foreground`.
3. **Status grid + Security Posture row** — `flex flex-col md:flex-row gap-4`:
   - Left: `flex-1 grid grid-cols-1 sm:grid-cols-2 gap-3` of 6 `StatusCell`s (Email, MFA, Services, API Keys, Nodes, Approvals). Each cell is its own `rounded-xl border border-border/50 bg-card px-4 py-3` link with a 32×32 icon tile, group-label role (`text-12 font-medium`, uppercase), body-role value (`text-14 font-normal`).
   - Right: `AccountPostureCard` — `w-[280px] rounded-xl border border-border/50 bg-card`, header (icon + "Security Posture" + colored status label), 4-item checklist with semantic icon coloring, footer with `0 of 4` counter and a `h-1.5` progress bar filled with `nyx-gradient-vivid`.
4. **Shortcuts** — `text-15 font-semibold` heading "Shortcuts", followed by a `grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3` of 5 `QuickActionCard`s (Services, API Keys, Nodes, Organizations, Channel Bots). Each card: `rounded-xl border border-border/50 bg-card px-3 py-4 text-center`, 32×32 icon tile, card-title role (`text-15 font-semibold`), secondary-caption role (`text-13 font-normal`).
5. **Right panel (registered via `setRightPanel`)** — `AiSetupCard` (dismissible, gradient/glow, "NEW" badge, primary CTA), `ApprovalsCard` (Telegram + mobile app), Quick Links card (Documentation, AI Setup Guide, Integration Guide). On `lg-`, this content is rendered inline at the bottom of the main column instead.

### Right Panel Cards (dashboard)
- **AI Setup Card:** `rounded-xl border border-nyx-500/30`, gradient background (`bg-gradient-to-b from-nyx-500/15 …` + radial top glow), "NEW" pill badge, dismissible (X top-right), `variant="primary"` CTA "Set up". Persists dismissal to `nyxid:ai-setup-dismissed`.
- **Approvals Card:** plain card, "Connect Telegram" (default button) + "Get App" (`variant="primary"`) stacked.
- **Quick Links:** plain card with an uppercase group-label heading (`text-12 font-medium`), list of `QuickLink` rows (action-link role, `text-14 font-medium`, plus `ArrowUpRight` icon and ghost hover).

### Get Started vs Quick Links
Get Started / Shortcuts = actionable setup steps inside the product. Quick Links = reference material (docs/guides). They must not overlap.

## Interaction Rules
- **Edit buttons:** Save/Cancel always positioned bottom-right of their edit container; Save is the rightmost button (`variant="primary"`), Cancel sits to its left
- **CTA disabled state:** Edit Save is disabled while pristine, invalid or pending; Create is disabled only while invalid or pending.
- **Tables with no actions:** Actions column must not be present if the table has no row actions
- **Tables with 2+ actions:** Use 3-dot dropdown menu instead of inline buttons (legacy violations: `approval-history`, `admin-user-detail` — fix when touching)
- **Status badges:** Always title-case (`Active`, `Pending`, not `active`, `PENDING`)
- **Hover states:** `bg-overlay` for subtle, `bg-overlay-strong` for interactive items
- **Mobile back navigation:** the top-bar logo zone becomes a back arrow when the current path is not a sidebar root; use `window.history.back()` rather than routing manually
- **Detail-page breadcrumb labels:** detail pages must call `useBreadcrumbLabel(name)` so the trailing crumb shows the entity name instead of the UUID

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-11-05 | Account text and range rows use deliberate shared layouts | Stack account text labels above left-aligned borderless inputs, provide explicit password placeholders and an editable-name affordance, and truncate long read-only email values beside their verification context. RangeControl aligns each label and current value above the slider with one muted helper, in both shells and mobile widths. MFA switches reflect the server state through setup/disable confirmation, announce pending/errors, and prevent dismissal during requests. |
| 2026-11-05 | Conversation previews require an intentional gesture | Passing pointers leave the rail quiet. Press and drag preview until release; keyboard focus retains its preview. Browser coverage includes virtualized-message jumps, isolated wheel scrolling and one roving tab stop. |
| 2026-11-05 | Account Settings shares inset grouped editing in both shells | Profile, Security, Sessions, MCP, Display and Privacy use sibling groups and external help. The panel header shares its background; visited segmented tabs retain drafts and one dirty-only viewport bar reviews profile/password edits without exposing passwords. Studio reuses the same forms; immediate actions and URL/focus/dismiss behavior stay intact. Mobile uses the full-width dialog sheet with keyboard-aware actions. |
| 2026-11-05 | Conversation index belongs to the chat viewport | Anchor to the transcript column before every preview/CMA/machine aside, independent of sidebar and saved column width. Fill the space below the toolbar and above the composer, retain 7px pitch and bounded scrolling with edge-aware fades, and change only the current tick’s accent color. Hide the rail below 768px without reserving a gutter. |
| 2026-11-04 | One control treatment and page rhythm in both shells | Shared themed controls and portals use the same density, padding, gaps, minimums, icons and radii. WorkspacePageHeader composes PageHeader; content insets, section gaps and card gaps share shell classes. Assistant-only root CSS is limited to transcript tick slots. Browser parity covers real shells and portals at all supported audit scales. |
| 2026-11-05 | Roles own line boxes and form captions | One field-label implementation serves all themed forms, including custom editor captions. Shared role utilities own size, weight and line height; source checks reject local overrides. Service summaries share header, description gap and two-line reservation. Actual create dialogs, settings forms and full service descriptions join the cross-shell parity matrix. |
| 2026-11-04 | Custom role and enlarged-text guarantees | Custom onboarding actions are text-14/500 and prominent analytics values are text-28/600. The shared 24px hit-area floor covers custom Studio actions as well as primitives. Toast actions and analytics filters reflow; chart axes refresh cached font measurement after Display changes. Active recipes reference the semantic role table. |
| 2026-11-04 | Canonical text numbers and shared semantic roles | Every text-N is N/16rem, defined once. A simultaneous markup/CSS migration preserves the current reading sizes; a separate change corrects semantic roles using the table above. Studio, NyxBot, auth/docs, charts, Billing, onboarding and portals use the same roles and weights. Root 0.85 rebase and spacing preferences are unchanged. Touch fields floor the input role at 16px; shared targets floor at 24px. |
| 2026-11-04 | One reading scale across Studio and NyxBot | The themed root owns the reading tokens and 1.5 body line height, so route changes and portals cannot change a token’s rendered size. Shared navigation icons follow their labels in both shells. The later one-treatment decision also makes control sizing and density global to themed surfaces. Never-themed landing/blog defaults remain separate. |
| 2026-11-04 | Rebase default text size to the previous 0.85×; restore dense rail and compact save chrome | The root baseline applies consistently in Studio and the assistant. Versioned migration preserves custom rendered sizes within bounds without changing spacing. Explicit rail geometry prevents general control scaling from stretching tick slots; compact bars and single-line reviews keep editing chrome small. |
| 2026-11-04 | Compact titles, explicit change review and viewport save actions | Observer-driven headers retain search focus and compensate their height in the scroll content. One bar outside the scrolling viewport preserves access to dirty forms across tabs, detail pages and mobile keyboards. Allowlisted field diffs make edit saves reviewable without exposing credentials; section help adds searchable context. |
| 2026-11-04 | Agent drawer search uses explicit live section registrations | Search groups results by tab and reveals exact rows or retained detail pages, preserving dirty drafts, save fences and dismissal guards. Search stays above the content region; keyboard and reduced-motion behavior are part of the pattern. |
| 2026-11-04 | Remaining workspace forms share grouped editing and dismissal guards | Automation, agent, crew, settings and machine editors use sibling groups, field-linked footer errors and primary submit actions. Dialog footers stay visible; inline settings bars appear while dirty. Dialogs isolate save scopes; route guards follow each active form. Existing paid quote acceptance and saved-request recovery stay explicit. Grouped variants remain explicit; shared control treatment now follows the later one-treatment decision in both shells. |
| 2026-11-04 | Workspace page family and equal-size plugin cards | Shared headers and segmented status views align workspace pages. Connectors and skill results share a text-scaled card size with native full-card actions and contained secondary controls; attached skills remain grouped rows. |
| 2026-11-04 | NyxBot inset grouped forms and segmented navigation | One neutral card per group, inset hairlines, borderless controls and external helper text replace nested boxes. Mounted tabs preserve drafts; detail navigation keeps independent revision-fenced saves. The subtle `border-border/50` edge preserves light-theme contrast. |
| 2026-04-01 | Initial design system created | Created by /design-consultation + /design-shotgun. User reviewed 7 directions and chose refined purple with warm neutrals. |
| 2026-04-01 | Keep purple, shift accent | User chose to keep purple identity but shift from #8b5cf6 to #9775fa (warmer, less saturated). |
| 2026-04-01 | Sidebar reorganization | Collapsed from 20 items to functional zones. Approvals unified into single nav group. |
| 2026-04-01 | Activity-first dashboard | Stats with meaningful descriptions, not just numbers. |
| 2026-05-11 | Global compact density | Shrunk all base components (buttons h-10→h-8, inputs h-10→h-8, tables, badges, switches, tabs, dialogs, dropdowns) for visual cohesion with tighter sidebar. |
| 2026-05-11 | 3-mode sidebar | Expanded (200px), Collapsed (52px), Expand on hover. Mode persisted in localStorage. |
| 2026-05-11 | Logo icon only | Sidebar no longer shows logo — logo icon moved to top bar. Always icon, never full wordmark in the app chrome. |
| 2026-05-11 | Top bar full-width | Top bar spans entire viewport width above sidebar + content, not nested inside the content column. |
| 2026-05-11 | Supabase-inspired dashboard | Status grid + posture card + quick actions layout. Right panel for promo/links. |
| 2026-05-11 | No overlap between Get Started and Quick Links | Get Started = actionable setup steps. Quick Links = reference material. |
| 2026-05-15 | Reintroduced sidebar group labels | The 9px uppercase "Approvals" / "Developer" / "Admin" labels make the four-zone structure scannable; collapsed mode falls back to a short divider line so the visual rhythm survives. |
| 2026-05-15 | Status grid is separate cards, not gap-px hairlines | Each status cell is its own `rounded-xl bg-card` link with `gap-3` between cells. Cleaner click targets, easier to scan. |
| 2026-05-15 | Rename "Get Started" → "Shortcuts" on dashboard | The "Getting started" name is now reserved for the first-run onboarding checklist that only appears before the first key. The persistent grid is "Shortcuts". |
| 2026-05-15 | Per-user onboarding takeover | First-run wizard renders in place of the dashboard chrome until the user finishes; no separate route. (PR #757) |
| 2026-05-15 | Codified `PageHeader`, `AddCtaButton`, list/detail patterns | Every non-dashboard page is built from these. Documenting them so new pages converge instead of reinventing. |
| 2026-05-15 | Page titles are responsive (22px / 28px) | `PageHeader` ships `text-22 sm:text-28`. Mobile downshift is intentional — never override. |
| 2026-05-15 | Tab content gap is 24px, two valid spellings | Real tab usages converge on a 24px gap between trigger row and content, expressed as either `mt-6` on each `TabsContent` (e.g. `keys.tsx`, `org-detail.tsx`) or `space-y-6` on the parent `<Tabs>` wrapper (e.g. `settings.tsx`, `consents.tsx`). Both are accepted; the older `mt-3` only survived in primitive demos. |
| 2026-05-15 | Admin pages share dashboard chrome | `adminLayout` is a child of `dashboardLayout` — no separate admin layout. Admin nav group is gated by `hasAdminRead` (admin or operator). |
| 2026-05-15 | `variant="primary"` is the standard dialog submit | The purple gradient is now the dominant action treatment in dialogs and right-panel CTAs. The "color is earned" principle still rules backgrounds, hover states, and idle surfaces. |
| 2026-05-15 | Reconciliation pass | Verifier flagged residual inaccuracies in the rewrite: MobileNav footer is a `User` icon tile not an avatar, mobile nav rows have no fixed-size active square, tab content uses either `mt-6` on `TabsContent` or `space-y-6` on `<Tabs>`, the right panel is conditionally mounted (not just hidden), and the ambient status line defaults to healthy when the user has no nodes. Added a Naming reconciliation table for sidebar/page-title/tab-label drift and a multi-list-page note (channel-bots stacks two list recipes). |
| 2026-07-22 | Assistant chat asymmetric layout | Chat thread refresh (Manus-inspired). User turns render as a subtle right-aligned bubble (`bg-overlay-strong`, `rounded-xl` with a sharpened `rounded-br-[2px]` tail corner, no icon); assistant turns render left with no bubble beside an 18px brand tile (icon only, no name label). Icon placement is **container-query** driven (`@container` on the thread scroller, `@min-[680px]:` on the row): the mark sits in a left gutter while the chat window is at least the thread's natural width (680px), and stacks on top of the answer once the window is squeezed narrower. Queried against the thread container, not the viewport, because the assistant surface sits beside a sidebar. Per-turn `HH:MM` mono timestamp from `message.created_at` travels with the icon and is hover-revealed (`opacity-0 group-hover:opacity-100`, plus `group-focus-within` for keyboard) to keep the resting state clean. A very-thin `h-4` scroll fade (`pointer-events-none absolute bottom-0 bg-gradient-to-t from-background`) sits at the base of the thread so the last turn dissolves into the composer instead of hard-cutting. |
| 2026-07-22 | Assistant turns render as one grouped voice | A turn streams as several `AssistantMessage`s (text, tool run, more text); `ChatThread` now groups consecutive same-role messages so the whole turn renders under a **single** identity mark with its blocks (tool runs included) stacked in the shared content column — no repeated NyxID icon per message. User messages group into stacked bubbles. |
| 2026-07-22 | Streaming caret + full markdown | The actively-streaming text block ends in a blinking caret (`animate-pulse`, glued inline via `[&>p:last-of-type]:inline`), with a standalone caret for the brief pre-first-block window, so a live turn reads as typing rather than a frozen partial. `TextBlock` now renders the full safe markdown set (lists, headings, blockquote, fenced code, `hr`, `del`) instead of flattening them — streamed results display in full. |
| 2026-07-22 | Tool activity persists (supersedes "transient whisper") | `RunCard` is now a per-run ledger that defaults to **expanded**: every step keeps its status icon and label, and `TOOL_CALL_END` only settles a step to done/failed (it never removes the step, and the run closes on `RUN_FINISHED`), so a finished tool call stays on screen rather than vanishing. The reader may collapse a run to a one-line summary, but it never auto-collapses on completion (that reads as "ended"). Overrides the earlier principle that tool activity should disappear as it settles. |
| 2026-07-30 | Assistant transcript and composer share one grid | Supersedes the responsive icon stacking in the 2026-07-22 chat-layout decision: the transcript and composer use the same `758px` outer grid, split into a fixed `30px` icon gutter and an adjacent content column up to `680px` wide. Assistant cards, user bubbles, and the composer occupy that content column at every viewport width; assistant prose and composer text share the same `7px` inset inside it. The NyxID mark stays in the left gutter without shifting the answer. Transcript and long-draft scroll regions use subtle scrollbars plus edge-aware top/bottom fades. The compact composer keeps single-line drafts and its measured controls inline; at 95% of the text space remaining beside those controls, or after an explicit newline, the text spans the first row and the controls move to a bottom-right row. The measurement always reserves the live controls width, so adding options scales without a hardcoded text width. The composer rail also subtracts the transcript's measured native scrollbar width, keeping cards, prose, and input text aligned when a desktop scrollbar consumes layout space. |
| 2026-07-31 | Pre-answer loader is three dots lighting in turn (supersedes the Newton's cradle) | The cradle it replaces was legible as a cradle — 7px swings, asymmetric gravity easing, a brightness handoff modelling kinetic energy, and an exit sweep that read each ball's live transform so it kept flying the way the impulse was already travelling. That was the problem: a loader is furniture in front of an answer and this one asked to be watched. Now **three** 6px dots on an 11px pitch (a 28px group, within 2px of the pack it replaces, so nothing around it re-flows). The 5px gap is nearly a full dot of air: these are three marks taking turns, not a chain in contact. Over a 1400ms period each dot is above rest for the first 30% of its own run (420ms, peaking at 196ms) and the stagger is **280ms — two thirds of the pulse**, peaks land 280ms apart at 196/476/756ms and each dot is back at rest 140ms before its neighbour peaks, so **at every peak the other two are at rest**. The only overlap is a 140ms tail where one decays and the next rises through roughly the same value — a handover, not a chord. The **pulse width** is the load-bearing number: the first attempt held each dot above rest for half its run (700ms) against a 180ms stagger, and at the middle dot's peak both neighbours sat near 0.82 — all three lit, a glow rather than a wave. Narrowing the pulse is what makes the peaks exclusive; widening the stagger to match is what spreads the relay over enough of the period to leave a rest gap after it. The relay ends at 980ms and the group rests together for the remaining 420ms — a breath, not the stall that reads as a dead turn. Contrast is unchanged and still load-bearing — `muted-foreground` cycling 65%→100%, so a resting dot clears 3:1 in both themes (3.8:1 dark, 3.2:1 light). The exit fades the **group**, not the dots: compounding the parent's opacity leaves each dot as bright as it was and still pulsing as it dissolves, where a per-dot exit would interpolate from the flat resting opacity and snap a dot caught at its peak dark for a frame. `StreamingDots` still pins the last measured box and drops it out of flow first, so the answer moves up immediately instead of waiting out the fade. Reduced motion keeps the synchronised 65%→85% breathe. |
| 2026-07-31 | The assistant composer holds focus by default | Arriving at the chat, opening a conversation, and finishing a turn all leave the caret in the composer — it is the only thing anyone opens this screen to use. Focus has one unconditional trigger and three conditional ones. Unconditional: an explicit `focusRequest` counter the page bumps from the sidebar row, New Chat, and deleting the conversation you were in. That takes focus outright, including off the sidebar row just clicked, because that click IS the request. Conditional (each requires focus to still be parked on the body, so nothing anyone is holding is taken): a `draftKey` that moves without a request — browser Back/Forward, canonical-id repair, confirmed-stale cleanup, the first send migrating off a draft thread; and a turn ending, which additionally restores only focus the turn itself took (the composer is read for focus during the render that flips `active`, before the commit disables the field). Parked focus is not by itself evidence that nobody is reading — scrolling, selecting answer text and a screen reader's virtual cursor all leave `activeElement` on the body — so from the moment a send starts (not from `active` — a first send can sit in `sending` for seconds) the composer watches for `pointerdown`/`keydown`/`wheel`/`touchstart` outside itself and drops any focus it was holding for you once you have moved on. That watch only exists while a turn runs, so the parked test stands alone for the automatic `draftKey` moves — acceptable because those are canonical-id repair and stale-route cleanup, which land moments after a conversation opens rather than at an arbitrary time. A request that lands while the field is disabled is held until the turn hands it back, and dropped if you move on first. Touch devices opt out entirely (`(pointer: coarse)`): raising the on-screen keyboard over half the screen is the wrong default when opening a conversation to read it is the common case. Known gap: the shell's home logo is a plain `Link`, so clicking it from a conversation leaves focus on the logo in browsers that focus links on click. |
| 2026-10-15 | Message timestamps name the date and zone inline (supersedes the `HH:MM` stamp in the 2026-07-22 chat-layout decision) | Every user message, reply, NyxBot instruction and group message shows its stored instant through the shared `MessageTime` (`components/assistant/message-time.tsx`): JetBrains Mono 11px `text-text-tertiary`, formatted with the reader's own locale and time zone and always naming the date, year and zone (`Oct 15, 2026, 3:40 AM GMT+8`), so messages from different days, or either side of a clock change, never read alike. Everything is inline; there is no tooltip. A reply's time ends its feedback line, and a user message's sits under its bubble. Both are revealed on hover or focus within the message (`group/message`) and always shown on devices without hover. Group agent replies keep the time in their header, always visible. A streaming reply shows none until it settles, and a missing or invalid instant renders nothing. |
| 2026-10-28 | Message timestamps show the day and month; reply feedback asks after the vote (supersedes the inline label of the 2026-10-15 timestamp decision) | `MessageTime` now shows only the local day and month (`27 Oct`), adding the year outside the reader's current year (`27 Oct 2025`), in the surrounding sans `text-11 text-text-tertiary` instead of JetBrains Mono. English readers always get day-first order; other languages keep their own (`10月27日`). The full date, time and zone moves to the native `title`, so it is still one hover away. Placement and reveal rules are unchanged. A reply's feedback line is copy, thumbs up, thumbs down and the time, with no Comment button. A vote saves at once and opens a standard `Dialog` (`md:max-w-md`; full screen on mobile like every dialog) from `components/assistant/reply-feedback.tsx`, shared by chats and groups, modelled on Claude's feedback dialog: title `Give positive feedback` or `Give negative feedback`, a description asking what the reader liked or what went wrong, `h-8 rounded-full text-12 font-medium` reason pills (a `hairline-strong` ring at rest; chosen pills gain a leading check, a `nyx-secondary-400` ring and a `/15` tint of it), a `Details (optional)` `Textarea`, and Cancel (`outline`) beside Submit (`primary`). Up offers Useful, Accurate and Easy to understand; down offers Not useful, Not accurate, Bad response and Hard to understand. Submit stores `Reasons, in offered order — details`, closes the dialog and confirms with a `Thanks for the feedback.` toast; an empty Submit just closes, since the vote is already saved. Cmd/Ctrl+Enter submits from the details field. Cancel, Escape, the close button or the overlay keep the vote; a failed Submit keeps the dialog and its text open with an error. Clicking the chosen thumb again withdraws the vote. |
| 2026-11-04 | Native voice mode is a full-screen call with a live aura | Supersedes the native voice sheet with its in-call voice and microphone pickers and two-caption “Show all” transcript; those choices move to Settings › Voice. The aura (`voice/VoiceAura.tsx`) is five soft `react-native-svg` radial-gradient discs in the theme's violet stops (light mode adds a white core), animated only through Reanimated transforms and opacity so the gradients never re-render. It breathes slowly (4.2s) at rest; Listening grows and brightens with the microphone level, Speaking with playback loudness, and Thinking (a queued or claimed task, or a finished utterance awaiting the reply) widens a slow 16s orbit. Levels come from Web Audio analysers in the hidden media WebView for both OpenAI and Grok (about 20 Hz, sent only on change) and ease on the UI thread with a 90ms attack and 320ms release, so the aura never jitters; the Speaking phase uses a 600ms quiet hold between words. Reduce Motion keeps a static glow whose opacity alone distinguishes phases. The call view is an opaque `overFullScreen` modal rather than `fullScreen`, because a fullScreen presentation detaches the screen hosting the media WebView, which would end the call. |
| 2026-10-29 | User chat bubble shape and editable colors | Supersedes the bubble fill and corners in the 2026-07-22 chat-layout decision: user messages in individual and group chats share three 18px corners and a tighter 4px bottom-right corner (pt on native). On web, dedicated `chat-bubble` and `chat-bubble-foreground` colors are editable per theme in Settings → Display with a live preview and a 4.5:1 text contrast check. Native Display offers device-saved Gray, Lavender, and Graphite palettes, each with a light and a dark version and live previews of both; every version meets 4.5:1 for text, including author, recipient, date, and copy controls. Gray is the default: light `#E0E0E5` with `#18181B` text, dark `#2C2C30` with `#EFEFF1` text. Dark Lavender is `#3A2E5E` / `#EEE8FF` and dark Graphite `#4A4A52` / `#FFFFFF`. |

## Portaled overlay layers

UI Dialog, Sheet, Select, Popover and DropdownMenu surfaces inherit their parent
layer through `OverlayLayer` React context, which follows portals. Each nested
surface adds 10; its backdrop sits one below it. AssistantShell establishes base
80 (the mobile drawer), so dialogs/sheets use 90 and their menus use 100 without
per-control z-index classes. Nested pickers and menus continue the same rule.
Tooltips use at least 1000 and stay above their parent. Embedded raw Radix modals
must provide their content layer to descendants. Avoid per-instance z-index
classes on these primitives; date pickers and comboboxes inherit through their
Popover/Select surface.

### Native assistant workspace

The React Native workspace has a base sidebar and an opaque content slab. Home,
chats and workspace pages push horizontally inside that slab. Only Settings,
account pages and QR/terminal quick actions use the retained pull-up surface. See
[the complete native route inventory](mobile/docs/NATIVE_WORKSPACE_ROUTES.md).

- The sidebar uses sentence-case headings and Quick actions above Workspace.
  Workspace contains Automations, Machines, Plugins, Activity (live pending count)
  and Create new agent; no coming-soon destinations. Activity shows Incoming
  followed by date-grouped History, with Active grants as a secondary page.
  Logo and the pinned Chat pill open Home. The Settings gear and Quick actions
  present above the current sidebar/slab without changing its offset or scroll.
  Their dismissal restores that exact state. Root selection fades the old content
  out over 120ms, swaps with no inner stack animation, and waits for the new page's
  layout and paint. It then fades the new content in over 180ms while closing the
  opaque slab in a single 300ms ease-out. Only the 24pt root edge reveals it;
  only the exposed slab sliver closes it. Finger releases spring; reduced motion
  has no travel. The pinned controls have a measured bottom content spacer.
- At window widths of 900pt and above, a persistent 300pt sidebar sits beside
  the same retained content navigator. Both columns remain touch- and
  accessibility-accessible; the sidebar does not own modal focus or a closing
  scrim. Narrower windows use the push interaction above, with the sidebar capped
  at 360pt. Resizing changes layout without replacing the chat or its draft.
- Drawer top is the window's safe-area top plus 10pt. On windows at least 768pt
  wide, it becomes a centered card up to 720×860pt with 24pt plus safe-area margins
  and rounded bottom corners. The keyboard resizes the measured available host;
  the panel fits that space without remounting navigation. Settings content stays
  within a centered 720pt column, and row wrapping uses actual measured row width.
  Top corners are 24pt; the
  centered 36×4pt handle has a 44pt drag region. A dedicated translucent scrim
  stays fixed over retained chat and fades independently while the panel slides.
  Sidebar and content headers share a 60pt row below the same safe-area inset.
  Page and Settings titles are centered across the full header, with equal control
  columns, 18pt type, one line and a 1.25 scaling cap. Chat identity keeps its avatar
  and name alignment. Close is a circular 44pt target. Only pages pushed within a
  sheet add Back; directly opened QR, Terminal or account pages are local roots.
  Sidebar accessibility follows the settled UI endpoint; cancelled or zero-distance
  gestures release its focus ownership and restore the content at offset zero.
  Forms retain async dirty/pending guards across every dismissal path. Reduced
  motion removes stack travel and springs. No fake home indicator or native rail.
- Home is an explicit primary-chat state, reached from the navigation logo/Chats,
  bare `/assistant`, or app start without a restored chat. New chat with NyxBot
  shows the same normal empty chat as a specialist: identity header, intro and
  composer. Compose actions preserve the viewed agent and start empty. Each
  thread keeps its own in-memory text/attachment draft when visiting Home or
  another thread; Home has a separate NyxBot composer.
- Native private and crew chats share `ChatSurface`: 8pt row/message gaps,
  10pt vertical message padding, 20pt history separators and header clearance,
  and the same user turn with a 38pt leading inset, 14/10pt bubble padding,
  18/18/4/18 corners and the selected accessible bubble palette. Private chat
  omits redundant “You”; crew author names/avatars, recipient pills, member
  subtitle and header options remain. Crew header options use the same 23pt
  MoreHorizontal affordance as private chat; no duplicate crew-settings control
  occupies the composer. Private per-agent permissions remain a labelled
  dropdown because they set execution mode, rather than crew membership. User prose shares 14/23pt body typography;
  agent prose, headings, links, code, activity and attachments share their
  production components without a crew markdown override. Both composer docks
  share 12pt margins, the saved column width, and the same frame/input/toolbar.
  Pending crew cards scroll in the transcript column, so jump-to-latest and
  edge fades always anchor directly above the composer on both surfaces.
  Native reply footers show only an icon Copy action and a local day/month date,
  8pt apart and 8pt below the body. Reply feedback votes are omitted on native by
  user decision (2026-10-28); full timestamps remain accessible on long press.
  Assistant prose stays unboxed. All chat composers share a filled 26pt pill:
  full-width text sits above a toolbar with Attach, optional mentions, dictation
  and a trailing Send/Stop circle inside a 44pt target. Attach offers Camera,
  Photos and Files through their native pickers. Dictation uses Apple's speech
  recognizer (on-device when the iPhone supports it) in the iPhone's language;
  no audio goes to NyxID. It never sends a message automatically and never
  opens a sheet or window: a tap or long press dictates at once, iOS asks for
  microphone and speech recognition access itself the first time, and the
  language is chosen in Settings › Voice (Dictation for accounts without live
  voice). While it runs it stays inside the composer pill: the existing draft (secondary) and the recognized words
  (interim words in tertiary) appear at the field's 14/20pt above a streaming
  waveform line that takes the place of the placeholder (3pt bars, one per
  ~90ms native volume sample, silent samples as dots), between an outlined
  Discard circle and a filled Finish circle, with a mono timer. Finish adds
  the words to the draft, where they are edited and sent like typed text.
  Idle errors stay a compact notice above the field. The jump-to-latest arrow
  dips 6pt and springs back as the transcript scrolls to the newest message
  (immediate under Reduce Motion). Chat pages
  use an edge-to-edge transcript viewport within the system safe area, so its
  scroll indicator sits at the window edge. The centered transcript content
  retains 12pt horizontal padding and the configured column maximum; composers
  retain their own 12pt margins and pages keep 4pt bottom padding. The navigation account footer absorbs its bottom clearance into
  the safe-area inset rather than adding a second padded strip. The one-row
  private header stays compact; group name, ownership context and settings
  share one header, with member details retained.
- Native auth screens scope their light palette locally: #F1F1F4 filled
  provider pills and fields, #E5E5EB pressed fill, #82828D control/divider edges,
  and #60606A placeholders and quiet labels. A focused field uses a single
  #79649F edge with no added glow or geometry change; error edges take priority.
  Text meets 4.5:1 and control edges meet 3:1 on normal/pressed fills and the
  white canvas. Sign-in, device codes, Agent Key login and MFA share this
  treatment. Workspace tokens, dark auth styling and the existing faint brand
  halo remain unchanged.
- Native pending attachment chips keep their width and text-scaled height through
  waiting, upload, checking, ready and error states, with a 44pt remove target.
  A thin bottom bar shows upload progress; waiting/checking gently pulse a short
  segment (static under Reduce Motion). Status text owns accessibility. Ready
  chips show only the thumbnail/icon, filename and remove control, without a
  tick or Ready label. Errors keep red text and remove the loader.
- Chat Details, History and Working context are nested pages in the content slab.
  History shares the cursor-paged Recents browser, filtered to the current agent.
  Title edits, deletion and context reset retain dirty/critical-write guards.
  The agent/permissions/options and recipients/connections pickers remain task-local
  transient choices, alongside media preview and voice conversation.
- Native permissions controls contain Full access, Auto and Ask every time only,
  with the effective agent override shown read-only. Chat and agent settings use
  the server default without a native selector or per-turn override. Voice automatically uses an available server default, without a source selector. This is the user's deliberate native scope (2026-10-28).
  Voice, conversation mode and completion notices are chosen in Settings › Voice;
  the call screen has no voice/source pickers. Voice composers never show pricing or tariffs, including connection, idle, minimized and help-popover states (user decision 2026-11-06, superseding V1). Pricing belongs in Settings › Voice. Hold to talk and Hands-free remain selectable through the compact header radio toggle: phone and radio glyphs, inactive icon only, active icon plus a one-line label, and 44pt minimum targets. Mode changes animate the header width, switching instantly under Reduce Motion. The status/timer has its own full-width line, with elapsed time protected from status truncation. At accessibility text sizes the agent name moves below the top control row; the selected mode label fits within the remaining width. xAI shows only the forced Hold to talk option and cannot switch modes. Changes still save the existing voice preference (user decision 2026-11-06, U11). Hold to talk keeps a press-and-hold microphone, release-to-send instruction and halo in both expanded and minimized layouts; tapping the minimized row opens the mode switch. Resizing releases an existing hold without changing modes.
- Native Automations is display-only: configured tasks, source/schedule, target,
  confirmation policy, read-only status and paged run history with exact thread
  links. It issues GET requests only. Creation, changes and execution belong to
  NyxBot; no native editor, preferences, toggle, invoke, delete or webhook setup.
- Reply activity/receipts and prose have a 16pt gap. Ordinary Markdown blocks have
  12pt sibling spacing; headings have 20pt (h1) or 16pt above and 8pt below, without
  start/end margins. Horizontally scrolling tables include the transcript's 12pt
  trailing inset. Private, live and crew replies share the same layout.
- Native skills are read-only except explicit version pinning and “Remove from
  agent”. “Use latest (version)” resolves the newest supported package and pins
  its exact version/hash; no floating latest contract exists. Editing belongs to
  NyxBot (“To change this skill, ask NyxBot.”). Owner publication decision cards
  retain their reviewed files and guarded approve/decline actions, without an editor.
- Settings → Display stores chat layout on this device with AsyncStorage. Column
  maximum defaults to 758pt, clamps to actual space, and is adjustable only on
  windows at least 600pt wide (360–1200pt range). Composer maximum defaults to the
  existing native 120pt (80–320pt preference range) and caps against available
  keyboard/window space, reserving transcript space down to a 44pt input. It
  grows with content; the setting never forces blank height. Reset changes only
  these layout values. Web storage and width/index behavior are unchanged.
- Settings preserves system text scaling. At narrow effective widths, identity,
  status, label/value and switch rows stack so long text can wrap within cards.
  Live system text-size changes refresh Settings and Display label measurements
  while retaining the page, scroll position and control state.
  Avatar initials retain enough space at the selected text size. Display step
  controls use fixed-size icons in labelled 44pt targets while labels and values
  continue to scale; the controls can wrap onto another line.
- Navigation, transcript, multiline input and horizontal attachment, recipient
  and member strips show fades only at measured overflowing edges. Top/start cues are gentler.
  Fades ignore pointer/accessibility input and disappear at revealed edges.
  Transcript measurement calls the existing follow handlers first and retains
  their drag/momentum behavior and anchors.
- Mobile muted/tertiary tokens were corrected across their shared consumers:
  dark `#8b839f`/`#827a96`, light `#746981` for both. Light status and tinted-brand
  text were darkened. These affect timestamps, metadata, placeholders, section
  labels and icon affordances throughout native management/chat; they no longer
  rely on faint text to express secondary hierarchy. Primary branding and web
  tokens are unchanged. Text colors have at least 4.5:1 against the native
  base/card/cardSoft surfaces; essential drawer/composer controls meet 3:1.

Native sign-in and the session-restore splash keep decoration restrained: a
faint, still violet halo behind the portal mark (`BrandNebula`, hidden from
VoiceOver) and a neutral soft shadow under the mark; no stars, drifting light
or coloured glow (user decision 2026-11-04). The splash shows only the mark
and one quiet line ("Signing you in…"). Sign-in has no boxed card: the hero
and the sign-in options rise gently into place on arrival (immediate under
Reduce Motion), a 30pt Space Grotesk "Welcome to NyxID" with NyxID in the
on-tint violet, a two-line lead, provider rows as 52pt soft filled pills
(mark and label centred, pressing in slightly), "or continue with email",
52pt rounded fields with leading mail and lock icons and a show/hide password
toggle, and Sign in, which always uses the primary gradient (user decision
2026-11-06) with a white label (user decision 2026-11-06). In light mode
empty/blocked forms keep the full gradient, since a dimmed gradient over white
drops the white label to ~2:1; they drop the glow and press-in and announce
disabled to assistive technology. In dark mode they dim only the gradient to 0.5
opacity behind the same white label. Ready uses full primary; pending keeps full primary
with a spinner and a busy/disabled state. MFA, device-login and Agent Key primary
submit buttons share this rule. One-time login codes sit centred in their own panel (mono
30pt, letter-spaced) with Copy, a live "Expires in m:ss · works once" line and
the terminal command in a monospace chip.

Native supports portrait iPhone, native iPad portrait/landscape and multitasking
windows, and Android phone with wide Android windows as a secondary layout. Eligible private chats expose a
voice-conversation mode in the trailing composer button when the server enables
`assistant:voice`: an empty composer shows the waveform, text or attachments
show Send, and an active reply shows Stop. Voice mode is a composer, not a
page: it slides up in place of the message composer (immediately under Reduce
Motion), so the chat above, with its action cards, approvals and connector
links, stays on screen and usable during the call. The voice composer uses the
composer frame wrapped in an animated pastel aura (soft blush and mint glows
and a thin gradient edge) that brightens and swells with the agent's voice,
follows yours while listening and breathes while thinking (still under Reduce
Motion). Expanded voice is content-sized, with no reserved caption height or text-entry row (user decision 2026-11-06). It scrolls only when its content reaches the current window cap, reserving space for the transcript on phones, iPad and split windows. Its header shows the agent name, status and call
timer, with the mode toggle and a 44pt “?” help control. Speaker mute sits beside the microphone controls. The compact anchored help popover explains the selected listening mode and echo/headphones guidance, has explicit close, outside-tap and accessibility escape dismissal, and contains secondary “Resume audio”. The media bridge exposes playback watermarks and loudness but no recoverable blocked/stalled signal: silence alone cannot distinguish a stalled player from a silent agent, so recovery stays in help (user decision 2026-11-06, U10). Only
unsealed user speech appears prominently in the voice panel, at 18/26pt in primary ink. Agent captions do not share this space. Sent speech, completed replies, task receipts and approval cards belong in the transcript (user decision 2026-11-06). While the call has agent tasks, one collapsed 44pt task chip (“1 task in progress”, “Needs your OK”) sits under the live words; tapping it lists each task’s outcome with request text behind Details, “Review in chat” for approvals and “Stop task” for queued or running work. It is the only mobile control that can cancel a voice task, so the #2321 fold keeps it; outcomes and cards still land in the transcript (fold decision 2026-11-10, pending review). Tapping the
voice entry starts the connection immediately using the saved voice and mode.
The timer shows “-” until connected, then counts from 00:00 in both modes.
Private voice requires both the verified active control snapshot and provider
readiness before reporting connected or starting its clock.
The private voice picker uses an opaque `overFullScreen` selection overlay,
keeping the underlying media host mounted and avoiding native page-sheet
detent changes. Its opening choices and selected voice stay fixed while it is
open; settings/options refreshes apply on the next opening, and Apply still
checks the live account, chat, source and available choice. Normal app
backgrounding ends local voice quietly. On reopening, bounded read-only
checks verify retained active or unknown call metadata before showing it as a
continuing blocker. Closing and unverified calls are distinct from a verified
active call; only one live call per person is allowed, and a verified terminal
or empty account slot permits a fresh explicit start. Checking never ends
another call or reconnects the microphone. When a person enters voice while
this chat's acknowledged End is still closing, show “Disconnecting previous
call…” and wait up to 30 seconds with bounded read-only checks. A confirmed
user-ended call or empty slot permits one “Starting new call…” attempt using
the current preferences. A verified active call requires the explicit “End
previous call and start here” action. Leaving, backgrounding, going offline,
changing account/chat/source/voice, a different active session, or a real error
cancels the queued restart. Idle, time-limit and other terminal reasons require
an explicit new start. App foregrounding alone never reconnects the microphone.
Hands-free listens automatically; Hold to talk retains its explicit microphone
control and release-to-send instruction. Holding swells the microphone and
lifts a soft halo (still under Reduce Motion).

Touching or scrolling the transcript minimizes voice to a single composer row.
The minimized expand control has a minimum 44×44pt touch target in both modes.
Voice is speech-only in both sizes: there is no typing field or text-send control (user decision 2026-11-06). Only tapping the minimized row expands it, without replacing the media engine or ending the session. Ending voice restores the normal composer with its existing draft intact. The minimized row uses the normal composer’s 4pt frame insets and single-line content height (one text line with 12/10pt field padding and its 44pt toolbar; crew includes its 36pt-or-text-scaled recipient strip and 6pt gap), with waveform, a mono timer and a 44pt End target centered vertically. Live words use one readable line in the expand target. Compact Hold to talk puts live words/timer above its instruction, waveform and 44pt microphone/End controls, within the same normal-composer height. Empty iOS composers use a one-line visual placeholder and expose its full prompt as an accessibility hint, so a wrapped placeholder cannot enlarge the field at accessibility text sizes. The same 3pt-bar dictation waveform appears in ordinary dictation and both private/crew voice sizes, driven by native mic levels and private playback levels. Reduce Motion shows a still dotted line and keeps the aura still. Closing voice or leaving
the chat ends audio; accepted agent tasks keep running. The aura is decorative
and hidden from VoiceOver (see the 2026-11-04 decision).

Crew composers share Attach, mentions, dictation review and toolbar controls
with private chats, plus recipients and crew settings. Empty crew composers
expose Crew voice. Because realtime provider sessions exclude group/member
threads, Crew voice uses native speech recognition with utterance boundaries,
posts spoken text through the ordinary crew sender to the displayed recipients,
and reads new agent replies with native speech synthesis. Each utterance binds
to the recipients displayed when capture starts. Opening the recipient picker
discards capture before presenting the picker; capture resumes after the
selection settles. It releases the
recognizer during playback, pauses for an unavailable crew, and
ends on backgrounding or leaving the chat. Uncertain delivery retains the
existing review-before-resend fence. This mode starts on entry and supports the
same voice shell, header, central microphone, End control and minimized row.
Silence re-arms listening; temporary app inactivity resumes automatically while
an explicit microphone pause stays paused. Reply speech uses prose and link labels,
with code omitted.
Ordinary composer dictation continues to insert words into the draft for
explicit review and sending.

Mobile onboarding entry points are temporarily commented out by user decision 2026-11-06 (U12), on iOS and Android. The home makes no onboarding requests and exposes no auto intro, Replay/Retry intro or onboarding-sourced “Suggested for you”. Restore the tagged `TODO(onboarding)` blocks together. The completed implementation and its unit tests remain for later work; the following describes that retained implementation, which is currently unreachable from the app.

The retained onboarding uses a native live replay of the same portable fixture timeline as the web intro (`shared/onboardingReplay.ts`). Real mobile chat turns, Markdown replies, agent avatars, confirmation cards and composer controls render at the current text size and theme; no desktop image is scaled into a phone. The replay fills the measured readable column, bounded at 100% with no intrinsic image width. Its transcript scrolls within min(380pt, 44% of the current window height), keeping the header and preview composer visible on phone, iPad and split windows. Native text reflows at its actual size instead of inheriting a recording aspect ratio. Timelines loop after a 3.5s pause, stop in the background and on unmount, and show the finished scene under Reduce Motion. No native dependency or WebView is added. The three obsolete GIFs are removed (user decision 2026-11-06).

The first-run page sheet constrains its safe-area host and scroll content to the actual sheet width, with a centered 680pt maximum readable column. “Not now” is deliberately aligned at the top-right in that same column, separate from the scrolling footer. Progress uses five equal 4pt-high segments with 6pt gaps across the column, clearly marks the selected step in primary ink, and retains labelled tab semantics. A 20pt hit extension above and below supplies 44pt targets without introducing visual gaps. Content scrolls vertically at large text sizes; the replay and sheet never expand horizontally (user decision 2026-11-06, U8/U9).

The native Machines drawer supports personal machines and organization machines
the signed-in person administers. It includes inventory, read-only local
capability and isolation details, guarded owner policy and update actions,
saved-login metadata and removal, activity, and revisioned per-agent machine
access. Context changes open the exact owner action card for review. Managed
and hosted setup, saved-login creation, and external API-key/OAuth grants use
web handoffs. Desktop entries from machine detail and private-chat tool cards
open a full-screen route with machine, conversation, context and display
selectors preserved. The viewer requires native privacy protection and a
compatible server; live-frame, performance and app-switcher verification remain
release gates. On iPad the viewer follows the current window without acquiring
a full-screen orientation lock; iPhone retains landscape entry and portrait
restoration. RN-web remains outside native support.
