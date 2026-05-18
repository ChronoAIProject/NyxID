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
- Purple accent ONLY on: active navigation icon color (`text-nyx-secondary-400`), sidebar control active dot, the `variant="primary"` button (gradient — used as the dominant action in dialogs, onboarding, and right-panel cards), AI setup card glow, role badges (`accent` variant for owner), logo
- Purple NOT on: idle surfaces, every hover state, surface tints on idle cards, decorative backgrounds. Hover stays neutral (`bg-white/[0.03]` / `bg-white/[0.06]`).
- Buttons: Approve = success green background tint, Deny / destructive = error red background tint, Secondary = ghost with border, Primary CTA = `variant="primary"` (`nyx-gradient-vivid`) — used on dialog submits, onboarding CTA, and right-panel promo cards
- The ambient status line at the top of the viewport is colored by node-fleet health: success gradient when healthy, warning when draining, destructive when offline (`AmbientStatusLine`, 2px, fixed top)

## Typography
- **Display/Hero:** Space Grotesk 500 — page titles, stat values, card headings. Techy and sharp.
- **Body:** Manrope 400 — all body text, descriptions, row content. Geometric but warm.
- **UI/Labels:** Manrope 500 — nav items, button labels, form labels
- **Data/Tables:** JetBrains Mono 400 — timestamps, log entries, API paths, code snippets. Supports tabular-nums.
- **Logo:** Playfair Display 400 — NyxID wordmark only (used on landing/legal pages; the logged-in app shows only the icon). Letterspace: 1px. Color: `#c4b5fd`.
- **Loading:** Google Fonts CDN

### Scale (in-app)
| Size | Use |
|------|-----|
| 9px | Sidebar group labels (uppercase, tracking 1.5px, `text-text-tertiary/50`) |
| 10px | Section labels (uppercase, tracking 1.5px), badge text, smallest overline text |
| 11px | Timestamps, stat descriptions, tertiary text, status labels, pagination counters |
| 12px | Body text, button text, nav items, table cells, input/select text, dropdown items, detail row values, breadcrumbs, page descriptions |
| 13px | Sidebar nav items, card body text, `DetailSection` headers, mobile-card primary text |
| 14px | Long-form welcome/marketing copy (e.g. onboarding takeover body, mobile nav items) |
| 15px | Dialog titles, card headings ("Shortcuts", "Getting started"), wizard step titles |
| 22px | Page titles on mobile (auto-scales to 28px at `sm`), `text-lg` mid-page section headings (`channel-bots`, `org-detail`) |
| 28px | Page titles on `sm+` (`PageHeader`, `font-bold`, `letter-spacing: -0.03em`); dashboard greeting |

> **Off-scale sizes in use** (not endorsed; reconcile when touching the file): `text-2xl` on `StatCard` values in `developer-apps.tsx`; `text-[22px] font-bold` on node-detail metric values; `text-3xl md:text-5xl font-normal` on the `developer-app-detail` not-found heading; `text-base` on `ApprovalSetupWizard` `CardTitle`; `text-[9px]` badges on `channel-conversation-detail`. New code should snap to the scale above.

## Components

### Buttons
- **Height:** default `h-8`, sm `h-7`, lg `h-9`, icon `h-8 w-8`
- **Text:** `text-[12px] font-medium`
- **Radius:** `rounded-lg` (8px)
- **Icon size:** `size-3` (12px) inside buttons
- **Gap:** `gap-1.5` between icon and text
- **ButtonIcon:** `h-[18px] w-[18px] rounded-[4px]` — small inset icon container for primary/destructive buttons
- **Variants:** default (ghost border), primary (gradient — used as the dominant dialog/CTA submit), destructive, outline, secondary, ghost, link
- **Loading:** `isLoading` prop shows `Loader2` spinner + children, disables button. Some legacy pages still use a label-swap (`{mutation.isPending ? "Creating..." : "Add"}`) — prefer `isLoading` for new code.
- **Rule:** Submit/Save buttons must be disabled when no changes exist or required fields are empty
- **`AddCtaButton`** (shared) — the standard "create" trigger on every list page header. `h-8`, `rounded-lg`, `border-white/[0.08]` idle / `border-white/[0.15]` hover, `text-[12px] text-text-tertiary`, with a 22×22 inset icon container (`rounded-[6px]`, `border-white/[0.08]`, `bg-white/[0.04]`, 12px Plus icon). Distinct from `variant="primary"` — used when the button is the page-level "add" affordance.

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
- **Variants:** `default` (purple — `border-nyx-500/30 bg-nyx-500/15 text-nyx-200`), `secondary` (`bg-muted`), `destructive`, `success`, `warning`, `info`, `accent` (purple, slightly softer fill)
- **Pattern:** `border-{color}/30 bg-{color}/15 text-{color}` (success/warning/info use `/10` fill)
- **Role badges** (`RoleBadge`): owner → `accent`, admin → `info`, member → `success`, viewer → `secondary`. Use these mappings consistently for any role/permission UI.

### Tables
- **Head height:** `h-8`
- **Head text:** `text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary`
- **Head padding:** `px-3`
- **Cell padding:** `px-3 py-2.5`
- **Cell text:** `text-[12px] text-foreground`
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
- **Trigger text:** `text-[12px]`
- **Active state:** `font-medium text-foreground` + 2px bottom indicator line
- **Content margin:** Two equivalent patterns are in use — either `mt-6` on each `TabsContent` (e.g. `keys.tsx`, `org-detail.tsx`) or `space-y-6` on the parent `<Tabs>` wrapper (e.g. `settings.tsx`, `consents.tsx`). Both produce the same 24px gap between trigger row and content. Pick one per page; the legacy `mt-3` only survives in primitive demos.
- **Indicator:** animated sliding `bg-primary` bar

### Dialogs
- **Padding:** `p-5`
- **Gap:** `gap-4`
- **Radius:** `rounded-xl`
- **Title:** `text-[15px] font-semibold`
- **Description:** `text-sm text-muted-foreground`
- **Close button:** top-right, `h-4 w-4` X icon
- **Submit button:** `variant="primary"` is the default treatment for the affirmative action; Cancel is `variant="outline"` or `variant="ghost"` to its left

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
- **Breadcrumbs:** rendered inline left of the actions on `md+`. `text-[12px]`; intermediate crumbs are links (`text-text-tertiary` → `text-foreground` on hover), the last crumb is plain `text-muted-foreground`. Detail pages register their own label via `useBreadcrumbLabel(label)`.
- **Actions (right):** Search trigger (opens command palette, shows `/` keybind hint), profile dropdown (icon button → menu with name/email/Settings/Log out), GitHub link. All actions use the same `border-white/[0.08]` chrome.
- **Mobile:** profile dropdown + hamburger (`Menu`) that opens the full-screen `MobileNav`

### Command Palette
Triggered by the search button or `/` key. Searches all sidebar destinations plus a small "actions" group ("Connect a Service", "Create API key", "Review approvals"). Items live in the shared `ALL_ITEMS` list in `command-palette.tsx`.

### Sidebar
- **3 modes:** Expanded (200px), Collapsed (52px), Expand on hover (52px in flow + 200px overlay)
- **Mode persistence:** `localStorage` key `nyxid:sidebar-mode`. The current width is also written to `--sidebar-width` so the top-bar logo zone can match it.
- **No logo** — logo lives in the top bar
- **Section labels** (when expanded): `text-[9px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50`, prefixed with horizontal padding `px-3 my-2`. Labels: "Approvals", "Developer", "Admin". The "Main" group has no label (it leads).
- **Collapsed mode:** label is replaced with a short divider line (`mx-auto w-3 border-t border-border/40`) so the visual separation survives.
- **Active nav item:** `bg-white/[0.06] font-medium text-foreground`, icon: `text-nyx-secondary-400`
- **Hover:** `bg-white/[0.03]`
- **Nav item:** `text-[13px]`, icon `h-[16px] w-[16px]`, `py-2` height, `gap-3 px-3` when expanded / `justify-center px-0 gap-0` when collapsed. Label transitions via `max-width: 0 → 160px` and `opacity: 0 → 1` (animated, never conditionally rendered, so layout never jumps).
- **Sidebar control:** bottom popover with 3 mode options, active option indicated by purple-filled `Circle` dot
- **Expand on hover:** outer `<aside>` is 52px in document flow, inner `<div>` is absolutely positioned and transitions width with shadow on the expanded state. 120ms enter delay, 250ms leave delay (via refs) to prevent flicker. Content is never blocked.

### Mobile Nav
Full-screen takeover (`fixed inset-0 z-[80]`) with `slide-in-from-bottom` enter / `slide-out-to-bottom` exit. Header with NyxID icon + wordmark + close button. Search input (`h-10 rounded-xl`) at top — typing filters the same `ALL_ITEMS` list as the command palette. Sections mirror the sidebar (Main → Approvals → Developer → Admin) with `text-[14px]` items in `rounded-xl px-4 py-3` rows; active state is a highlighted full-width row (`bg-white/[0.06] font-medium text-foreground`), not a fixed-size square. Footer: a user-info row (`h-8 w-8` icon tile holding a Lucide `User` glyph — not an avatar image — plus name and email) above two full-width split buttons (Settings / Log out, both `rounded-xl py-2.5`), safe-area-aware bottom padding.

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

### Naming reconciliation
The sidebar label and page title can drift; track this when writing breadcrumbs or copy:

| Sidebar | Page title (`PageHeader`) | Tabs (if any) |
|---------|---------------------------|---------------|
| AI Services | Services & Credentials (`keys.tsx`) | External Services, Agent Keys |
| Access & Auth | Access & Authorizations (`consents.tsx`) | Authorized Apps, Authorizations |
| Settings | Account Settings (`settings.tsx`) | Profile, Security, Sessions, MCP, Privacy |
| Nodes | Credential Nodes (`nodes.tsx`) | — |
| Guide | Setup Guide (`guide.tsx`) | — |
| Integration | Integration & SDK Guide (`integration-guide.tsx`) | — |
| AI Setup | AI Setup Guide (`ai-setup.tsx`) | — |

The breadcrumb labels in `dashboard-layout.tsx` (`SIDEBAR_ITEMS`) and the command-palette entries in `command-palette.tsx` (`ALL_ITEMS`) should match the page-title column above, not the sidebar column.

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
- Title: `text-[22px] sm:text-[28px] font-bold leading-none tracking-tight`, inline `letterSpacing: -0.03em`. The mobile downshift to 22px is intentional — never override.
- `leading` slot: 32–48px avatar/icon to the left of the title (`OrgAvatar`, status dot, color chip)
- `description`: `text-[12px] text-muted-foreground` directly below the title group
- `actions` slot: right-aligned, `flex items-center gap-2 shrink-0`. Most pages put their `AddCtaButton` here; filter pages put a `Select` here; multi-action pages may stack a couple of icon buttons.
- A few legacy pages (`provider-list`, `developer-apps`, `settings`, `guide`, `ai-setup`) still hand-roll a header. They typically also miss the `text-[22px] sm:text-[28px]` responsive downshift (e.g. `provider-list.tsx:129` and `ai-setup.tsx:266` both hardcode `text-[28px]`), so on mobile their titles overflow. New code uses `PageHeader`; when touching a legacy page, migrate it.

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
- **Mobile card anatomy:** `rounded-xl border border-border/50 bg-card p-4`, primary text `text-[13px] font-semibold`, metadata rows `text-[11px]`, single trailing action button (`h-7 w-7` or `h-8 w-8` ghost icon) absolutely positioned top-right; multiple actions go in a 3-dot dropdown.
- A few list pages (`service-list`, `provider-list`, `orgs`) use a **card grid** instead of a table: `grid gap-4 sm:grid-cols-2 lg:grid-cols-3`. Use a card grid when each item is more "tile-like" than "row-like" (has a distinguishing icon and ≤3 fields).
- **Multi-list pages:** some pages stack more than one list on a single route (e.g. `channel-bots.tsx` shows the bots list and the device-channels list one above the other). Each list keeps its own list-page recipe (mini PageHeader-style `text-lg` section heading → toolbar → responsive split). Don't tab-ify or accordion these unless the page would otherwise scroll past two screens.

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
- `DetailSection`: `rounded-xl border border-border/50 bg-card overflow-hidden` with a header strip (`border-b border-border/50 px-4 py-2.5`, title `text-[13px] font-semibold`) and `divide-y divide-border/30` body.
- `DetailRow`: `flex items-center justify-between px-4 py-2.5 text-[12px]`. Left = label in `text-muted-foreground`. Right = value (`font-medium text-foreground` by default, or a `Badge` if `badge` prop set), optionally followed by a `h-6 w-6` ghost copy button when `copyable`.
- For inline editable fields (display value + pencil → input + check/X), use the local pattern from `key-detail.tsx` (don't reinvent).
- For embedded forms inside a `DetailSection`, pad the form area with `p-5` and right-align the submit button (`flex justify-end`).

### Empty state
```
flex flex-col items-center justify-center gap-1 py-12 text-center
  ↳ <Illustration className="h-64 w-64 text-muted-foreground/30" />   (or h-48 w-48 inside a section)
  ↳ <p className="text-[12px] font-medium text-muted-foreground/30">Headline</p>
  ↳ <p className="text-xs text-muted-foreground/30">Optional supporting line.</p>
  ↳ <AddCtaButton ... />   (optional)
```

Illustrations live in `components/icons/empty-state/` — pick a thematically relevant one (mystery box, magic key, dish antenna, robot…). Never use a generic Lucide icon at this size.

### Error state
- Inline retryable errors: `<ErrorBanner message onRetry?>` from `components/shared/error-banner.tsx`. `rounded-xl border border-destructive/15 bg-destructive/[0.04] px-4 py-3`, 36×36 icon tile, message in `text-[12px] text-destructive`, optional `Retry` button on the right.
- Page-level fetch failure: same shape as the empty state, but with an error-flavored headline ("Failed to load X") and the same illustration tone.

### Code blocks
Two flavors, used everywhere docs/credentials/configs appear (`guide`, `ai-setup`, `integration-guide`, `key-detail`, `nodes`):

- **Block:** `<pre className="rounded-lg border border-border bg-muted px-4 py-3 pr-12 font-mono text-xs leading-relaxed">…`. Optional filename `Badge` above. Copy button absolutely positioned top-right (`absolute top-2 right-2 h-7 w-7` or `h-8 w-8`, ghost icon button, swaps to a green `Check` for ~2s after copy).
- **Inline chip:** `<code className="rounded bg-muted px-1.5 py-0.5 text-xs">snippet</code>` for short identifiers in prose.
- **`CopyableField`** (shared) — single-line value with a copy button. `rounded-xl border border-border bg-muted font-mono` body + absolutely-positioned `h-8 w-8` (or `h-7 w-7` `size="sm"`) copy button at right-center. Use for tokens, IDs, URIs displayed once.

### Settings rows
For toggle/select/input rows inside a settings card: `flex items-center justify-between rounded-lg border border-border p-4`, label + `text-[12px] text-muted-foreground` description column on the left, control on the right. Used in `notification-settings`, the form-row pattern in MFA/email settings, etc.

### Banners & callouts
- **Info / "important context"**: `rounded-xl border border-{color}/15 bg-{color}/[0.04] px-4 py-3` with a 36×36 `bg-{color}/10` icon tile and message text in `text-[12px] text-{color}` (or muted). Color picks: `success` (org-shared credential), `info`/`primary` (tip), `warning`/amber (pending action), `destructive` (error).
- **Pending-webhook / blocked state** (large, page-spanning): `rounded-xl border border-amber-500/30 bg-amber-500/10 p-6 text-center` with a centered illustration. Use when the user must take action elsewhere before the page becomes useful.
- **Empty/info microblock** (inline inside a card): `rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground`.

### Danger zone
Bottom of any settings tab that has a destructive action: a separate `Card` with `border-destructive/40`, `CardTitle className="text-destructive"`, description in `text-destructive/70`, action button right-aligned. Always visually segregated from the regular settings form above (don't mix destructive actions into the main form).

### One-time secret reveal
For client secrets, service-account credentials, and node registration tokens, use the `ClientSecretDialog` shape:
- Dialog title: "Save … Secret" / "New …"
- `DialogDescription`: "This … is shown only once. Copy and store it securely now."
- Each value rendered as a mono code block (`rounded-lg border border-border bg-muted px-3 py-2 font-mono text-xs`) with an inline `h-7 w-7` copy button
- Dismiss button: `variant="primary"` labeled "I have saved it" (not "Close" / "Done")

### Search bar
Used on every searchable list (admin lists, sessions, etc.):
```
flex items-center gap-2
  ↳ relative wrapper:
      Input  className="h-8 pl-9 text-[12px]"
      Search icon: absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-text-tertiary
  ↳ Button variant="outline">Search</Button>
  ↳ Button variant="ghost" if query>Clear</Button>
```

### Pagination
Used at the bottom of any paginated list. `flex items-center justify-between` row:
- Left: count text in `text-[11px] text-text-tertiary` ("Showing 1–25 of 137")
- Right: prev / "Page X of Y" / next group, prev/next as `h-8 w-8` outline icon buttons with `ChevronLeft`/`ChevronRight`

### Stat cards / mini-grid
For a row of small KPIs inside a card or section: `grid gap-3 grid-cols-2 sm:grid-cols-4` of `rounded-xl border border-border/50 bg-white/[0.02] p-4 text-center` tiles. Value is the dominant element (snap to a defined scale step — 28px or, where space is tight, 22px), label below in `text-[11px]`. The standalone `StatCard` used on `developer-apps` is a related but slightly different pattern that should be reconciled.

### Ambient status line
A 2px gradient bar pinned to the very top of the viewport (`fixed top-0 left-0 right-0 z-[60]`), driven by `useNodes()`. Healthy = success-green gradient; draining = amber; offline nodes present = destructive red, animated pulse. Defaults to **healthy green** when the user has no nodes at all — the bar is always present and never empty. Every page sees it; pages should not draw their own equivalent.

### Onboarding (first-run + dashboard checklist)
- **Takeover:** while `useShouldShowOnboarding()` reports the user hasn't completed AI-services onboarding, `DashboardLayout` renders `OnboardingTakeover` *in place of* the normal chrome (no separate route). Centered `max-w-md` column at `pt-[12vh]`, brand mark in a `rounded-2xl` purple-tinted tile, 28px welcome heading, 14px supporting copy, primary CTA, Skip link below. Both actions stamp the server-side flag before unmounting.
- **Checklist:** when activeKeys === 0, the dashboard renders `OnboardingChecklist` ("Getting started", N of 3 complete) above the greeting. Mobile = vertical timeline with connector line between icon tiles; desktop = three horizontal cards with a connector line through the icon row. Done states: green check + completed copy; in-progress: purple `nyx-secondary-400` icon + colored connector. Dismissible via a top-right X (persisted to `nyxid:onboarding-dismissed`).

---

## Dashboard Content Strategy
The dashboard (after onboarding completes) follows this composition top-to-bottom:

1. **Optional `OnboardingChecklist`** — only when no API keys exist; see Onboarding above.
2. **Greeting** — `text-[22px] sm:text-[28px] font-bold leading-[1.1]` inline `letterSpacing: -0.03em`, "Welcome back, {name}". Email below at `text-[12px] text-muted-foreground`.
3. **Status grid + Security Posture row** — `flex flex-col md:flex-row gap-4`:
   - Left: `flex-1 grid grid-cols-1 sm:grid-cols-2 gap-3` of 6 `StatusCell`s (Email, MFA, Services, API Keys, Nodes, Approvals). Each cell is its own `rounded-xl border border-border/50 bg-card px-4 py-3` link with a 32×32 icon tile, 10px uppercase label, 13px value.
   - Right: `AccountPostureCard` — `w-[280px] rounded-xl border border-border/50 bg-card`, header (icon + "Security Posture" + colored status label), 4-item checklist with semantic icon coloring, footer with `0 of 4` counter and a `h-1.5` progress bar filled with `nyx-gradient-vivid`.
4. **Shortcuts** — `text-[15px] font-semibold` heading "Shortcuts", followed by a `grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3` of 5 `QuickActionCard`s (Services, API Keys, Nodes, Organizations, Channel Bots). Each card: `rounded-xl border border-border/50 bg-card px-3 py-4 text-center`, 32×32 icon tile, 12px title, 10px description.
5. **Right panel (registered via `setRightPanel`)** — `AiSetupCard` (dismissible, gradient/glow, "NEW" badge, primary CTA), `ApprovalsCard` (Telegram + mobile app), Quick Links card (Documentation, AI Setup Guide, Integration Guide). On `lg-`, this content is rendered inline at the bottom of the main column instead.

### Right Panel Cards (dashboard)
- **AI Setup Card:** `rounded-xl border border-nyx-500/30`, gradient background (`bg-gradient-to-b from-nyx-500/15 …` + radial top glow), "NEW" pill badge, dismissible (X top-right), `variant="primary"` CTA "Set up". Persists dismissal to `nyxid:ai-setup-dismissed`.
- **Approvals Card:** plain card, "Connect Telegram" (default button) + "Get App" (`variant="primary"`) stacked.
- **Quick Links:** plain card with a 10px uppercase header, list of `QuickLink` rows (12px label + `ArrowUpRight` icon, ghost hover).

### Get Started vs Quick Links
Get Started / Shortcuts = actionable setup steps inside the product. Quick Links = reference material (docs/guides). They must not overlap.

## Interaction Rules
- **Edit buttons:** Save/Cancel always positioned bottom-right of their edit container; Save is the rightmost button (`variant="primary"`), Cancel sits to its left
- **CTA disabled state:** Submit/Save buttons disabled when no changes exist or required fields are empty
- **Tables with no actions:** Actions column must not be present if the table has no row actions
- **Tables with 2+ actions:** Use 3-dot dropdown menu instead of inline buttons (legacy violations: `approval-history`, `admin-user-detail` — fix when touching)
- **Status badges:** Always title-case (`Active`, `Pending`, not `active`, `PENDING`)
- **Hover states:** `bg-white/[0.03]` for subtle, `bg-white/[0.06]` for interactive items
- **Mobile back navigation:** the top-bar logo zone becomes a back arrow when the current path is not a sidebar root; use `window.history.back()` rather than routing manually
- **Detail-page breadcrumb labels:** detail pages must call `useBreadcrumbLabel(name)` so the trailing crumb shows the entity name instead of the UUID

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
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
| 2026-05-15 | Page titles are responsive (22px / 28px) | `PageHeader` ships `text-[22px] sm:text-[28px]`. Mobile downshift is intentional — never override. |
| 2026-05-15 | Tab content gap is 24px, two valid spellings | Real tab usages converge on a 24px gap between trigger row and content, expressed as either `mt-6` on each `TabsContent` (e.g. `keys.tsx`, `org-detail.tsx`) or `space-y-6` on the parent `<Tabs>` wrapper (e.g. `settings.tsx`, `consents.tsx`). Both are accepted; the older `mt-3` only survived in primitive demos. |
| 2026-05-15 | Admin pages share dashboard chrome | `adminLayout` is a child of `dashboardLayout` — no separate admin layout. Admin nav group is gated by `hasAdminRead` (admin or operator). |
| 2026-05-15 | `variant="primary"` is the standard dialog submit | The purple gradient is now the dominant action treatment in dialogs and right-panel CTAs. The "color is earned" principle still rules backgrounds, hover states, and idle surfaces. |
| 2026-05-15 | Reconciliation pass | Verifier flagged residual inaccuracies in the rewrite: MobileNav footer is a `User` icon tile not an avatar, mobile nav rows have no fixed-size active square, tab content uses either `mt-6` on `TabsContent` or `space-y-6` on `<Tabs>`, the right panel is conditionally mounted (not just hidden), and the ambient status line defaults to healthy when the user has no nodes. Added a Naming reconciliation table for sidebar/page-title/tab-label drift and a multi-list-page note (channel-bots stacks two list recipes). |
