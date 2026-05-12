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
- **Key principle:** Color is earned. Semantic colors (green/amber/red) do the heavy lifting for status. Purple marks identity and interaction, nothing else.
- **Density:** Compact — the entire UI skews small and tight. Elements are visually cohesive at reduced sizes. If in doubt, go smaller.

## Color

### Primary Accent
- **Primary:** `#9775fa` — warm violet (NOT #8b5cf6 which is the AI-default Tailwind violet-500)
- **Primary light:** `#c4b5fd` — for logo wordmark, light text on accent backgrounds
- **Primary deep:** `#7c5ce0` — for hover states, pressed states

### Backgrounds (3-layer depth)
- **Sidebar:** `#06060b` — darkest layer
- **Base:** `#07060e` — main background
- **Surface/Card:** `#0c0b14` — elevated content

### Text Hierarchy (4 levels)
- **Primary:** `#e8e4f0` — headings, important text (warm off-white, NOT pure white)
- **Secondary:** `#9e96b0` — body text, descriptions
- **Muted:** `#7a7490` — supporting text, metadata
- **Tertiary:** `#4a4460` — timestamps, disabled text, section labels

### Borders
- **Default:** `#1c1828` — card borders, dividers
- **Subtle:** `border-border/50` — lighter dividers within cards, section separators
- **Interactive:** `border-white/[0.08]` idle, `border-white/[0.15]` hover — for buttons, inputs, interactive chrome

### Semantic Status
- **Success/Online:** `#34d399` — active services, healthy nodes, approved grants
- **Warning/Expiring:** `#f59e0b` — expiring tokens, pending approvals, attention items
- **Error/Expired:** `#f87171` — expired tokens, failed requests, denied approvals
- **Info/Auth:** `#60a5fa` — informational badges, auth events

### Usage Rules
- Purple accent ONLY on: active navigation icon color (`text-nyx-secondary-400`), sidebar control active dot, primary CTA gradient buttons, AI setup card glow, logo
- Purple NOT on: primary action buttons (use semantic colors), every hover state, surface tints on idle cards, decorative backgrounds
- Buttons: Approve = success green background tint, Deny = error red background tint, Secondary = ghost with border, Primary CTA = `nyx-gradient-vivid` (rare, only for main page CTAs)

## Typography
- **Display/Hero:** Space Grotesk 500 — page titles, stat values, card headings. Techy and sharp.
- **Body:** Manrope 400 — all body text, descriptions, row content. Geometric but warm.
- **UI/Labels:** Manrope 500 — nav items, button labels, form labels
- **Data/Tables:** JetBrains Mono 400 — timestamps, log entries, API paths, code snippets. Supports tabular-nums.
- **Logo:** Playfair Display 400 — NyxID wordmark only. Letterspace: 1px. Color: `#c4b5fd`.
- **Loading:** Google Fonts CDN
- **Scale:**
  - 10px — section labels (uppercase, tracking 1.5px), badge text, smallest overline text
  - 11px — timestamps, stat descriptions, tertiary text, status labels
  - 12px — body text, button text, nav items, table cells, input text, select text, dropdown items, detail row values
  - 13px — nav items (sidebar), card body text, section titles in detail views
  - 15px — dialog titles, card headings, "Get started" section titles
  - 28px — page titles (`font-bold`, `letter-spacing: -0.03em`)

## Components

### Buttons
- **Height:** `h-8` (one standard size), icon `h-8 w-8`. No size variants in application code.
- **Text:** `text-[12px] font-medium`
- **Radius:** `rounded-lg` (8px)
- **Icon size:** `size-3` (12px) inside buttons
- **Gap:** `gap-1.5` between icon and text
- **ButtonIcon:** `h-[18px] w-[18px] rounded-[4px]` — small inset icon container for primary/destructive buttons
- **Variants:** default (ghost border), primary (gradient), destructive, outline, secondary, ghost, link
- **Loading:** `isLoading` prop shows `Loader2` spinner + children, disables button
- **Rule:** Submit/Save buttons must be disabled when no changes exist or required fields are empty

### Inputs
- **Height:** `h-8`
- **Text:** `text-[12px]`
- **Padding:** `px-3 py-1.5`
- **Radius:** `rounded-lg`
- **Border:** `border-input`, focus: `border-white/[0.15]`
- **Placeholder:** `text-text-tertiary`

### Selects
- **Trigger height:** `h-8`
- **Text:** `text-[12px]`
- **Padding:** `px-3 py-1.5`
- **Trigger radius:** `rounded-lg`
- **Item radius:** `rounded-md`
- **Item padding:** `py-1.5 pl-3 pr-8`
- **Chevron:** `h-3.5 w-3.5`

### Badges
- **Padding:** `px-2 py-0.5`
- **Text:** `text-[10px] font-medium`
- **Radius:** `rounded-md`
- **Variants:** default (purple), secondary, destructive, success, warning, info, accent
- **Pattern:** `border-{color}/30 bg-{color}/15 text-{color}`

### Tables
- **Head height:** `h-8`
- **Head text:** `text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary`
- **Head padding:** `px-3`
- **Cell padding:** `px-3 py-2.5`
- **Cell text:** `text-[12px] text-foreground`
- **Row border:** `border-b border-border`
- **Rule:** Actions column only present when the table has row actions. No empty "Actions" header.

### Switches
- **Track:** `h-5 w-9`
- **Thumb:** `h-4 w-4`
- **Checked translate:** `translate-x-4`

### Tabs
- **List height:** `h-8`
- **Trigger padding:** `px-3 py-2`
- **Trigger text:** `text-[12px]`
- **Active state:** `font-medium text-foreground` + 2px bottom indicator line
- **Content margin:** `mt-3`
- **Indicator:** animated sliding `bg-primary` bar

### Dialogs
- **Padding:** `p-5`
- **Gap:** `gap-4`
- **Radius:** `rounded-xl`
- **Title:** `text-[15px] font-semibold`
- **Description:** `text-[12px] text-muted-foreground`
- **Close button:** top-right, `h-4 w-4` X icon

### Dropdown Menus
- **Content radius:** `rounded-xl`
- **Content padding:** `p-2`
- **Item padding:** `px-3 py-1.5`
- **Item text:** `text-[12px]`
- **Item radius:** `rounded-md`
- **Item gap:** `gap-2`
- **Hover:** `bg-white/[0.06]`

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
- **Text:** `text-xs`

### Detail Sections
- **Container radius:** `rounded-xl`
- **Header padding:** `px-4 py-2.5`
- **Header text:** `text-[13px] font-semibold`
- **Row padding:** `px-4 py-2.5`
- **Row text:** `text-[12px]`
- **Dividers:** `divide-y divide-border/30`

## Spacing
- **Base unit:** 4px
- **Density:** Compact
- **Scale:** 2xs(2px) xs(4px) sm(8px) md(16px) lg(24px) xl(32px) 2xl(48px)
- **Content padding:** `px-6 pt-6 md:px-8 lg:px-10` (main area)
- **Card padding:** `p-4` (standard), `p-3` (compact/quick links)
- **Nav item padding:** `py-2`, expanded: `gap-3 px-3`, collapsed: `justify-center px-0`
- **Gap between sections:** `gap-8` (dashboard), `gap-4` (card groups)
- **Gap between cards:** `gap-3` (right panel), `gap-4` (status grid)
- **Gap within cards:** `gap-3` (title to content), `gap-2.5` (between rows)

## Layout

### Shell Structure
Top bar spans full width. Sidebar + content sit below it. Sidebar does NOT span full height — it sits under the top bar.
```
+--------------------------------------------------+
|  [logo] [breadcrumbs]    [search] [user] [gh]   |  <- TopBar (52px, full width)
+--------+-----------------------------------------+
|        |                                         |
| Side   |  Main Content         | Right Panel     |
| bar    |                       | (280px, opt.)   |
|        |                       |                 |
+--------+-----------------------------------------+
```

### Top Bar
- **Height:** `h-[52px]`
- **Logo:** NyxID icon (`h-5 w-5`), left-aligned with `mr-4`
- **Breadcrumbs:** Auto-derived from route path, after logo
- **Actions:** Search, Profile dropdown, GitHub link — right-aligned
- **Border:** `border-b border-border/60`

### Sidebar
- **3 modes:** Expanded (200px), Collapsed (52px), Expand on hover (52px flow + 200px overlay)
- **Mode persistence:** `localStorage` key `nyxid:sidebar-mode`
- **No logo** — logo lives in the top bar
- **Section dividers:** `mx-2 my-2 border-t border-border/40` between nav groups (lines don't touch sidebar edges)
- **No section labels** — groups are separated by divider lines only, no "Approvals" / "Developer" headings
- **No page-level breadcrumbs** — breadcrumbs are only in the top bar, auto-derived from the URL. PageHeader does not accept a breadcrumbs prop.
- **Active nav item:** `bg-white/[0.06] font-medium text-foreground`, icon: `text-nyx-secondary-400`
- **Hover:** `bg-white/[0.03]`
- **Sidebar control:** bottom popover with 3 options, active option indicated by purple-filled `Circle` dot
- **Expand on hover:** outer `<aside>` is 52px in document flow, inner `<div>` is absolutely positioned at `z-30` and transitions width. Overlays content — does NOT push main column. Shadow (`shadow-xl shadow-black/20`) on expanded state. Enter delay 120ms, leave delay 250ms.
- **Nav label transition:** labels always rendered with `opacity-0 w-0` when collapsed (no conditional rendering — prevents layout jumping)

### Right Panel
- **Width:** `w-[280px]`
- **Visibility:** `hidden lg:flex`
- **Card gap:** `gap-3`
- **Padding:** `px-3 pt-6 pb-6`

### Border Radius
- **Cards, panels:** `rounded-xl` (12px)
- **Buttons, inputs, nav items:** `rounded-lg` (8px)
- **Dropdown items, select items:** `rounded-md` (6px)
- **Badges:** `rounded-md` (6px)
- **Tooltips:** `rounded-[6px]`
- **Popovers, dialogs:** `rounded-xl` (12px)

## Navigation Structure
Sidebar organized into 3 groups with divider lines:

**Main**
- Dashboard, AI Services, Organizations, Nodes, Channel Bots, Settings, Authorized Apps, Authorizations, Guide

**Approvals**
- Notifications, Approval History, Active Grants

**Developer**
- Developer Apps, AI Setup, Integration Guide

**Admin** (separate admin layout, admin-only)
- Users, Audit Log, Service Accounts, Roles, Groups, Nodes, Services, Providers, Invite Codes

## Motion
- **Approach:** Minimal-functional
- **Easing:** enter(`ease-out`) exit(`ease-in`) move(`ease-in-out`)
- **Duration:** micro(50-100ms) short(150-200ms) medium(200-300ms)
- **Where used:** nav hover/active transitions (`duration-200`), dropdown open/close, sidebar width transition (`transition-[width] duration-200`), skeleton loading, tab indicator sliding (`duration-300`), card hover glow
- **Sidebar hover:** 120ms enter delay, 250ms leave delay (via refs, prevents flicker)

## Error / Not-Found States
- **Pattern:** `PageHeader` with entity name + `ErrorBanner` with message and retry
- **Never** use centered icon + text + back-button layouts for error states
- **ErrorBanner:** `rounded-xl border-destructive/15 bg-destructive/[0.04]`, icon in `h-9 w-9 rounded-lg bg-destructive/10`, retry button
- **Example:** `<PageHeader title="Node Not Found" />` + `<ErrorBanner message="..." onRetry={refetch} />`

## Dashboard Content Strategy
Supabase-inspired layout with status overview + security posture + quick actions:

1. **Greeting** — "Welcome back, {name}" at 28px bold, email below at 13px muted
2. **Status grid** — 2x3 individual cards (Email, MFA, Services, API Keys, Nodes, Approvals) with icon + label + value. Each cell is its own `rounded-xl border border-border/50 bg-card` card with `gap-3` between them. Not a monolithic bordered block.
3. **Account posture card** — 280px side card with security checklist (4 items with icons), progress bar, percentage score
4. **Get started** — 5 quick action cards (Services, API Keys, Nodes, Organizations, Channel Bots) in responsive grid
5. **Right panel** — AI setup promo card (dismissible), Approvals card (Telegram + mobile app), Quick Links (Documentation, AI Setup Guide, Integration Guide)

### Right Panel Cards
- **AI Setup Card:** gradient border/glow, dismissible, "NEW" badge, primary CTA button
- **Approvals Card:** Telegram connect + mobile app download
- **Quick Links:** Documentation, AI Setup Guide, Integration Guide — no overlap with Get Started items

## Cards
- **CardTitle:** Uses the component's default size. Never add `text-base` or `text-lg` class overrides.
- **CardTitle icons:** Do not put inline icon `<span>` elements inside CardTitle. If a card needs an icon, place it outside the title.
- **Destructive cards:** When a card has a single destructive action (e.g. Delete Account), place the button inline with the header row (`flex-row items-center justify-between`), not in a separate CardFooter.
- **Code blocks:** All `<pre>` blocks use `rounded-lg border border-border bg-muted px-4 py-3` with a copy button (`absolute right-2 top-2`).

## Interaction Rules
- **Edit buttons:** Save/Cancel always positioned bottom-right of their edit container
- **CTA disabled state:** Submit/Save buttons disabled when no changes exist or required fields are empty
- **Tables with no actions:** Actions column must not be present if the table has no row actions
- **Tables with 2+ actions:** Use 3-dot dropdown menu instead of inline buttons
- **Status badges:** Always title-case (`Active`, `Pending`, not `active`, `PENDING`)
- **Hover states:** `bg-white/[0.03]` for subtle, `bg-white/[0.06]` for interactive items
- **Eye icon convention:** `EyeOff` (slashed) = password hidden, `Eye` (open) = password visible
- **Badge alignment:** Use `items-center` for badge + text rows, not `items-start`
- **FormLabel:** Never add `text-base` class override. Uses component default size.
- **Toggle switch layout:** When a switch sits inline with its label, override FormItem spacing with `!space-y-0` and use `items-center` for vertical centering.

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-04-01 | Initial design system created | Created by /design-consultation + /design-shotgun. User reviewed 7 directions and chose refined purple with warm neutrals. |
| 2026-04-01 | Keep purple, shift accent | User chose to keep purple identity but shift from #8b5cf6 to #9775fa (warmer, less saturated). |
| 2026-04-01 | Sidebar reorganization | Collapsed from 20 items to functional zones. Approvals unified into single nav group. |
| 2026-04-01 | Activity-first dashboard | Stats with meaningful descriptions, not just numbers. |
| 2026-05-11 | Global compact density | Shrunk all base components (buttons h-10->h-8, inputs h-10->h-8, tables, badges, switches, tabs, dialogs, dropdowns) for visual cohesion with tighter sidebar. |
| 2026-05-11 | 3-mode sidebar | Expanded (200px), Collapsed (52px), Expand on hover. Mode persisted in localStorage. |
| 2026-05-11 | Logo icon only | Sidebar no longer shows logo — logo icon moved to top bar. Always icon, never full wordmark in the app chrome. |
| 2026-05-11 | Top bar full-width | Top bar spans entire viewport width above sidebar + content, not nested inside the content column. |
| 2026-05-11 | Supabase-inspired dashboard | Status grid + posture card + quick actions layout. Right panel for promo/links. |
| 2026-05-11 | No overlap between Get Started and Quick Links | Get Started = actionable setup steps (Services, Keys, Nodes, Orgs, Bots). Quick Links = reference material (Docs, AI Setup Guide, Integration Guide). |
| 2026-05-11 | UI consistency sweep | CardTitle: no text-size overrides, no inline icon spans. Code blocks: always have copy button. Status grid: individual cards not monolithic block. Badges: items-center alignment. Eye icons: EyeOff=hidden, Eye=visible. Sidebar hover: z-30 overlay, not push. Delete card: button inline with header row. |
