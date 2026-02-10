# Architecture Plan: Modal-to-Page Navigation Redesign

## Overview

Convert the NyxID frontend from modal-based detail/edit flows to full-page views using TanStack Router nested routes. The Services section is the primary target - the `ServiceDetailDialog` and `ServiceEditDialog` modals become dedicated route-based pages. Quick-action modals (create, delete, endpoint CRUD) remain as modals.

## Current State Analysis

### Route Structure (flat)
```
rootRoute
├── authLayout (/login, /register)
└── dashboardLayout
    ├── / (DashboardPage)
    ├── /api-keys (ApiKeysPage)
    ├── /services (ServicesPage)         ← all detail/edit via modals
    ├── /connections (ConnectionsPage)
    └── /settings (SettingsPage)
```

### Modal-Based Flow (current)
1. User clicks `ServiceCard` on `/services`
2. `ServiceDetailDialog` opens as a modal overlay (max-w-2xl, max-h-85vh)
3. Inside the detail dialog, clicking "Edit" opens a second modal `ServiceEditDialog`
4. All state is held via `useState` on the `ServicesPage` component (`selectedService`, `detailOpen`)
5. URL never changes - no deep linking, no back-button support, no shareable URLs

### Key Issues
- Detail dialog is already at 446 lines with multiple sections (General, OIDC, Endpoints, MCP)
- Stacking two modals (detail + edit) is poor UX
- No URL-addressable detail views - users cannot bookmark or share a service detail link
- Credential state cleanup is manual (`handleClose` resets 3 booleans + removes query cache)
- Browser back button does nothing useful when modals are open

---

## New Route Architecture

### Route Tree
```
rootRoute
├── authLayout
│   ├── /login
│   └── /register
└── dashboardLayout
    ├── / (DashboardPage)
    ├── /api-keys (ApiKeysPage)
    ├── /services (ServicesPage layout with Outlet)
    │   ├── /services/ (index: ServiceListPage - the card grid)
    │   ├── /services/$serviceId (ServiceDetailPage)
    │   └── /services/$serviceId/edit (ServiceEditPage)
    ├── /connections (ConnectionsPage)
    └── /settings (SettingsPage)
```

### Route Parameters
- `$serviceId` - UUID string, validated in the route's `beforeLoad` or loader

### Data Loading Strategy
- `/services` index: `useServices()` hook (existing, no change)
- `/services/$serviceId`: `useService(serviceId)` hook (existing at `use-services.ts:24`)
- `/services/$serviceId/edit`: same `useService(serviceId)` hook
- Endpoints, OIDC credentials loaded as sub-queries on the detail page (existing hooks, no change)

---

## What Becomes a Page vs. Stays a Modal

### Becomes a full page
| Current Component | New Route | Reason |
|---|---|---|
| `ServiceDetailDialog` | `/services/$serviceId` | Complex multi-section view (General, OIDC, Endpoints, MCP). Benefits from full-page layout, URL addressability, back-button navigation |
| `ServiceEditDialog` | `/services/$serviceId/edit` | Full form that benefits from URL addressability and dedicated page real estate |

### Stays as a modal
| Component | Location | Reason |
|---|---|---|
| Create Service dialog | `ServicesPage` (list) | Quick 4-field form, no deep content |
| Delete Service button | `ServiceCard` | Single-click destructive action, inline confirm is sufficient |
| `EndpointFormDialog` | `ServiceDetailPage` | Quick CRUD form for sub-resource, does not need its own URL |
| `MfaSetupDialog` | Settings page | Wizard/setup flow that is transient |
| MFA disable dialog | Settings page | Confirmation dialog |
| `ApiKeyCreateDialog` | API Keys page | Quick form |

---

## Component Architecture

### New Files to Create

```
frontend/src/
├── pages/
│   ├── services.tsx                    ← MODIFY: becomes layout with Outlet
│   ├── service-list.tsx                ← NEW: extracted card grid (from services.tsx body)
│   ├── service-detail.tsx              ← NEW: full-page detail view
│   └── service-edit.tsx                ← NEW: full-page edit form
├── components/
│   ├── shared/
│   │   ├── breadcrumb.tsx              ← NEW: breadcrumb navigation component
│   │   ├── page-header.tsx             ← NEW: reusable page header with breadcrumb + actions
│   │   ├── detail-section.tsx          ← NEW: extracted from service-detail-dialog.tsx
│   │   └── detail-row.tsx             ← NEW: extracted from service-detail-dialog.tsx
│   └── dashboard/
│       ├── service-card.tsx            ← MODIFY: onClick navigates instead of opening modal
│       ├── service-detail-dialog.tsx   ← DELETE: replaced by service-detail.tsx page
│       ├── service-edit-dialog.tsx     ← DELETE: replaced by service-edit.tsx page
│       ├── header.tsx                  ← MODIFY: dynamic title from route context/breadcrumbs
│       ├── sidebar.tsx                 ← NO CHANGE (startsWith already handles nested routes)
│       ├── endpoint-list.tsx           ← NO CHANGE (already a standalone component)
│       ├── endpoint-form-dialog.tsx    ← NO CHANGE (stays as modal)
│       ├── redirect-uri-editor.tsx     ← NO CHANGE (already a standalone component)
│       ├── mcp-connection-info.tsx     ← NO CHANGE (already a standalone component)
│       └── oidc-credentials-section.tsx ← NEW: extracted OIDC section from detail dialog
```

### Files Modified

| File | Change |
|---|---|
| `router.tsx` | Add nested service routes with `$serviceId` param |
| `pages/services.tsx` | Slim down to layout shell with `<Outlet />` for nested routes |
| `components/dashboard/service-card.tsx` | Replace `onClick` callback with `useNavigate` to `/services/$serviceId` |
| `components/dashboard/header.tsx` | Replace static `getPageTitle` with dynamic breadcrumb-aware title |

### Files Deleted

| File | Replacement |
|---|---|
| `components/dashboard/service-detail-dialog.tsx` | `pages/service-detail.tsx` + extracted shared components |
| `components/dashboard/service-edit-dialog.tsx` | `pages/service-edit.tsx` |

---

## Detailed Component Designs

### 1. Router Changes (`router.tsx`)

```typescript
// New: services becomes a layout route
const servicesLayout = createRoute({
  path: "/services",
  getParentRoute: () => dashboardLayout,
  component: ServicesPage,  // Now just renders <Outlet />
});

const servicesIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => servicesLayout,
  component: ServiceListPage,
});

const serviceDetailRoute = createRoute({
  path: "$serviceId",
  getParentRoute: () => servicesLayout,
  component: ServiceDetailPage,
});

const serviceEditRoute = createRoute({
  path: "$serviceId/edit",
  getParentRoute: () => servicesLayout,
  component: ServiceEditPage,
});

// Route tree update
dashboardLayout.addChildren([
  dashboardIndexRoute,
  apiKeysRoute,
  servicesLayout.addChildren([
    servicesIndexRoute,
    serviceDetailRoute,
    serviceEditRoute,
  ]),
  connectionsRoute,
  settingsRoute,
])
```

### 2. ServicesPage Layout (`pages/services.tsx`)

Becomes a thin layout wrapper. The create-service dialog and page header move here so they're available on the list page, while the `<Outlet />` renders the active child route.

```
ServicesPage (layout)
└── <Outlet />
    ├── ServiceListPage (index: card grid + create dialog)
    ├── ServiceDetailPage ($serviceId)
    └── ServiceEditPage ($serviceId/edit)
```

Decision: The `ServicesPage` layout should be minimal - just `<Outlet />`. The page header and create dialog belong on `ServiceListPage` since they are only relevant when viewing the list, not when viewing a detail or edit page.

### 3. ServiceListPage (`pages/service-list.tsx`)

Extracted from current `services.tsx`. Contains:
- Page header ("Services" heading + "Add Service" button)
- Create service dialog (stays as modal)
- Card grid with `ServiceCard` components
- Loading/empty states

Key change: `handleServiceClick` replaced by navigation. `ServiceCard.onClick` navigates to `/services/${service.id}`.

### 4. ServiceDetailPage (`pages/service-detail.tsx`)

Full-page version of current `ServiceDetailDialog`. Sections:
- **Page header**: breadcrumb (`Services > {service.name}`) + Edit button (navigates to `/services/$serviceId/edit`)
- **General section**: slug, base URL, auth type, status, timestamps (using `DetailRow`)
- **OIDC Configuration section** (conditional): client ID, credential reveal, redirect URIs, discovery endpoints, regenerate secret
- **API Endpoints section**: `EndpointList` component (no change)
- **MCP Connection section**: `McpConnectionInfo` component (no change)

Data fetching: uses `useService(serviceId)` from route params. Shows loading skeleton while fetching.

Credential cleanup: When user navigates away (route change), React unmounts the component, which naturally cleans up state. For OIDC credential cache cleanup, use a `useEffect` cleanup or `onLeave` route callback to remove the query cache entry - matching current `handleClose` behavior.

### 5. ServiceEditPage (`pages/service-edit.tsx`)

Full-page version of current `ServiceEditDialog`. Contains:
- **Page header**: breadcrumb (`Services > {service.name} > Edit`)
- The edit form (name, description, base_url, api_spec_url, auth_type badge)
- Cancel button navigates back to `/services/$serviceId`
- Save button submits then navigates back to `/services/$serviceId`

Data fetching: uses `useService(serviceId)` from route params.

### 6. Breadcrumb Component (`components/shared/breadcrumb.tsx`)

Simple, reusable breadcrumb that takes an array of `{ label: string, to?: string }` items:

```typescript
interface BreadcrumbItem {
  readonly label: string;
  readonly to?: string;  // If undefined, rendered as current (non-link) item
}

interface BreadcrumbProps {
  readonly items: readonly BreadcrumbItem[];
}
```

Renders as: `Services / My Service / Edit` with all but the last item being clickable links.

### 7. PageHeader Component (`components/shared/page-header.tsx`)

Reusable header with breadcrumb + title + optional action buttons:

```typescript
interface PageHeaderProps {
  readonly breadcrumbs?: readonly BreadcrumbItem[];
  readonly title: string;
  readonly description?: string;
  readonly actions?: React.ReactNode;
}
```

### 8. OidcCredentialsSection (`components/dashboard/oidc-credentials-section.tsx`)

Extracted from lines 148-304 of `service-detail-dialog.tsx`. Self-contained component that manages its own state (showCredentials, secretVisible, confirmRegenerate) and cleans up on unmount.

```typescript
interface OidcCredentialsSectionProps {
  readonly serviceId: string;
  readonly oauthClientId: string | null;
}
```

### 9. Header Changes (`components/dashboard/header.tsx`)

The current `getPageTitle` function uses a static pathname-to-title map. This needs to handle dynamic nested routes.

Options:
- **Option A**: Parse the pathname and use route context/matches to build the title
- **Option B**: Use TanStack Router's `useMatches()` with route context to get the title from the deepest matching route

**Decision: Option B** - Each route defines a `context` with a `title` string. The Header reads the deepest match's context to display the page title. This is the idiomatic TanStack Router approach.

For detail/edit pages where the title includes the service name, the route's `context` can be set in the route's `beforeLoad` or the page component can provide it via a route context update.

Simpler alternative: Just use breadcrumbs in the page content area and keep the Header title showing the top-level section name ("Services"). The sidebar `isActive` check already uses `startsWith`, so `/services/abc` correctly highlights "Services" in the sidebar. The Header only needs to show "Services" for all nested service routes.

**Final decision**: Keep the Header simple - derive the top-level section title from the first path segment. Add breadcrumbs inside page content for nested navigation. This minimizes changes to the Header component.

```typescript
function getPageTitle(pathname: string): string {
  const titles: Record<string, string> = {
    "/": "Dashboard",
    "/api-keys": "API Keys",
    "/services": "Services",
    "/connections": "Connections",
    "/settings": "Settings",
  };
  // Match on first path segment
  const segment = "/" + (pathname.split("/")[1] ?? "");
  return titles[segment] ?? "Dashboard";
}
```

### 10. ServiceCard Changes (`components/dashboard/service-card.tsx`)

Replace `onClick` callback prop with internal navigation:

```typescript
// Before
interface ServiceCardProps {
  readonly onClick: () => void;
  // ...
}

// After
interface ServiceCardProps {
  readonly service: DownstreamService;
  readonly onDelete: (id: string) => void;
  readonly isDeleting: boolean;
  // onClick removed - card navigates internally
}
```

The card uses `useNavigate()` to go to `/services/${service.id}` on click.

---

## Implementation Phases

### Phase 1: Extract Shared Components (Low Risk)
1. **Create `components/shared/detail-section.tsx`** - Extract `DetailSection` from `service-detail-dialog.tsx:334-347`
2. **Create `components/shared/detail-row.tsx`** - Extract `DetailRow` from `service-detail-dialog.tsx:349-391`
3. **Create `components/shared/breadcrumb.tsx`** - New component
4. **Create `components/shared/page-header.tsx`** - New component
5. **Create `components/dashboard/oidc-credentials-section.tsx`** - Extract OIDC block from `service-detail-dialog.tsx:148-304`
6. **Create `components/dashboard/discovery-endpoints.tsx`** - Extract `DiscoveryEndpoints` from `service-detail-dialog.tsx:393-446`

All extractions are pure refactors - export the same interfaces, no behavior changes.

### Phase 2: Create New Page Components (Medium Risk)
1. **Create `pages/service-detail.tsx`** - Full-page detail view using extracted components + existing `EndpointList`, `McpConnectionInfo`
2. **Create `pages/service-edit.tsx`** - Full-page edit form (adapted from `ServiceEditDialog`)
3. **Create `pages/service-list.tsx`** - Card grid extracted from current `services.tsx`

These are new files that don't affect existing behavior yet.

### Phase 3: Wire Up Routes (Higher Risk - Breaking Change)
1. **Modify `router.tsx`** - Add nested service routes
2. **Modify `pages/services.tsx`** - Slim down to layout with `<Outlet />`
3. **Modify `components/dashboard/service-card.tsx`** - Navigate on click instead of callback
4. **Modify `components/dashboard/header.tsx`** - Update `getPageTitle` for nested routes
5. **Delete `components/dashboard/service-detail-dialog.tsx`**
6. **Delete `components/dashboard/service-edit-dialog.tsx`**

### Phase 4: Polish
1. Verify back-button behavior works correctly at each navigation level
2. Confirm OIDC credential cache cleanup on route leave
3. Verify sidebar active state for all nested routes
4. Test deep-link to `/services/{id}` directly (ensure auth guard works)
5. Test loading/error states when navigating directly to a service that doesn't exist

---

## Navigation & UX Behavior

### Back Button Behavior
| From | Back Button Goes To |
|---|---|
| `/services/$serviceId/edit` | `/services/$serviceId` |
| `/services/$serviceId` | `/services` (list) |
| `/services` (list) | Browser history (likely `/` or previous page) |

All handled automatically by browser history since we use `navigate()` which pushes to history.

### Breadcrumb Trails
| Route | Breadcrumbs |
|---|---|
| `/services` | (none - top-level page) |
| `/services/$serviceId` | `Services` > `{service.name}` |
| `/services/$serviceId/edit` | `Services` > `{service.name}` > `Edit` |

### 404 / Not Found Handling
If a user navigates to `/services/invalid-uuid`, the `useService()` hook will return an error. The `ServiceDetailPage` should handle this gracefully with a "Service not found" message and a link back to `/services`.

---

## Risks & Mitigations

### Risk 1: OIDC credential cache leaking after navigation
- **Impact**: Decrypted secrets lingering in TanStack Query cache
- **Mitigation**: Add `useEffect` cleanup in `OidcCredentialsSection` that calls `queryClient.removeQueries()` on unmount, matching current `handleClose` behavior
- **Severity**: Medium (security-sensitive)

### Risk 2: Deep-link to detail page when not authenticated
- **Impact**: User gets redirected to login, but return URL is lost
- **Mitigation**: The existing `beforeLoad` guard on `dashboardLayout` already handles this. TanStack Router preserves the intended URL through the redirect flow if configured. Verify this works end-to-end.
- **Severity**: Low

### Risk 3: Service data not loaded when navigating directly to detail
- **Impact**: Brief loading state or flash of skeleton
- **Mitigation**: `useService(serviceId)` already handles this with TanStack Query. Show a proper loading skeleton. No route-level loader needed since the hook approach is already established.
- **Severity**: Low

### Risk 4: Breaking existing delete flow on service card
- **Impact**: Delete button on card currently calls `handleDelete` which checks if the deleted service matches `selectedService` and closes the detail dialog. With page-based navigation, if user deletes from the detail page, they should be redirected to the list.
- **Mitigation**: Delete from card (list view) stays the same. Delete from detail page should navigate to `/services` after successful deletion. Add `onSuccess` callback to `useDeleteService` or handle in the page component.
- **Severity**: Low

---

## Success Criteria

- [ ] `/services/$serviceId` renders full service detail as a page (not modal)
- [ ] `/services/$serviceId/edit` renders the edit form as a page (not modal)
- [ ] Browser back button navigates correctly at each level
- [ ] Deep-linking to `/services/{valid-id}` works (loads data, renders page)
- [ ] Deep-linking to `/services/{invalid-id}` shows "not found" gracefully
- [ ] OIDC credentials are cleaned from cache when leaving detail page
- [ ] Sidebar correctly highlights "Services" for all `/services/*` routes
- [ ] Header shows "Services" title for all nested service routes
- [ ] Breadcrumbs render correctly on detail and edit pages
- [ ] Create service modal still works from the list page
- [ ] Endpoint CRUD modals still work from the detail page
- [ ] No regressions on other pages (API Keys, Connections, Settings)
