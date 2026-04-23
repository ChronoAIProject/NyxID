/**
 * Access Scope card — Services + Nodes multi-select, porting
 * `.wizard-access-card` from `cli/src/wizard/assets/wizard.html:170-220`
 * and `cli/src/wizard/assets/wizard.css:552-600`.
 *
 * Each subsection has an "Allow all" master checkbox at the top. When
 * Allow-all is on, the list is hidden and the caller should send the
 * corresponding `allow_all_*: true` on the backend request. When
 * Allow-all is off, the list is visible; the caller sends the
 * selected ids plus `allow_all_*: false`.
 *
 * Fetches live data from the same endpoints the dashboard uses
 * (`/keys` for services, `/nodes` for nodes). Matches the v3.2 Mode A
 * switch from `/user-services` → `/keys` so rows expose both `label`
 * and `slug`.
 */

import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { useKeys } from "@/hooks/use-keys"
import { useNodes } from "@/hooks/use-nodes"

export interface AccessScopeState {
  readonly allowAllServices: boolean
  readonly allowAllNodes: boolean
  readonly selectedServiceIds: ReadonlySet<string>
  readonly selectedNodeIds: ReadonlySet<string>
}

export interface AccessScopeCardProps {
  readonly value: AccessScopeState
  readonly onChange: (next: AccessScopeState) => void
}

export function AccessScopeCard({ value, onChange }: AccessScopeCardProps) {
  const services = useKeys()
  const nodes = useNodes()

  function toggleService(id: string) {
    const next = new Set(value.selectedServiceIds)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    onChange({ ...value, selectedServiceIds: next })
  }

  function toggleNode(id: string) {
    const next = new Set(value.selectedNodeIds)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    onChange({ ...value, selectedNodeIds: next })
  }

  return (
    <section
      aria-labelledby="access-scope-title"
      className="flex flex-col gap-4 rounded-lg border border-border bg-muted/30 p-4"
    >
      <div className="flex flex-col gap-1">
        <h3 id="access-scope-title" className="text-sm font-semibold">
          Access Scope
        </h3>
        <p className="text-xs text-muted-foreground">
          Restrict which services and nodes this key can access via proxy.
        </p>
      </div>

      <AccessGroup
        label="Services"
        icon={<ShieldIcon />}
        allowAll={value.allowAllServices}
        onAllowAllChange={(allowAll) => {
          onChange({ ...value, allowAllServices: allowAll })
        }}
        listLabel="Select allowed services:"
        loading={services.isLoading}
        items={
          services.data?.map((s) => ({
            id: s.id,
            primary: s.label,
            secondary: s.slug,
          })) ?? []
        }
        selectedIds={value.selectedServiceIds}
        onToggle={toggleService}
      />

      <AccessGroup
        label="Nodes"
        icon={<ServersIcon />}
        allowAll={value.allowAllNodes}
        onAllowAllChange={(allowAll) => {
          onChange({ ...value, allowAllNodes: allowAll })
        }}
        listLabel="Select allowed nodes:"
        loading={nodes.isLoading}
        items={
          nodes.data?.map((n) => ({
            id: n.id,
            primary: n.name,
            secondary: n.status,
          })) ?? []
        }
        selectedIds={value.selectedNodeIds}
        onToggle={toggleNode}
      />
    </section>
  )
}

interface AccessGroupProps {
  readonly label: string
  readonly icon: React.ReactNode
  readonly allowAll: boolean
  readonly onAllowAllChange: (next: boolean) => void
  readonly listLabel: string
  readonly loading: boolean
  readonly items: ReadonlyArray<{
    readonly id: string
    readonly primary: string
    readonly secondary?: string
  }>
  readonly selectedIds: ReadonlySet<string>
  readonly onToggle: (id: string) => void
}

function AccessGroup({
  label,
  icon,
  allowAll,
  onAllowAllChange,
  listLabel,
  loading,
  items,
  selectedIds,
  onToggle,
}: AccessGroupProps) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-1.5 text-sm font-medium">
        <span className="text-muted-foreground">{icon}</span>
        <span>{label}</span>
      </div>
      <Label className="flex cursor-pointer items-center gap-2 text-sm">
        <Checkbox
          checked={allowAll}
          onCheckedChange={(checked) => {
            onAllowAllChange(checked === true)
          }}
        />
        <span>Allow all {label.toLowerCase()}</span>
      </Label>

      {allowAll ? null : (
        <div className="flex flex-col gap-1.5 rounded-md border border-border bg-background/40 p-3">
          <p className="text-xs text-muted-foreground">{listLabel}</p>
          {loading ? (
            <p className="text-xs text-muted-foreground">Loading…</p>
          ) : items.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              None available. Add one first, then come back.
            </p>
          ) : (
            <div className="flex flex-col gap-1" role="list">
              {items.map((item) => (
                <Label
                  key={item.id}
                  className="flex cursor-pointer items-center gap-2 text-sm"
                >
                  <Checkbox
                    checked={selectedIds.has(item.id)}
                    onCheckedChange={() => {
                      onToggle(item.id)
                    }}
                  />
                  <span className="truncate">
                    {item.primary}
                    {item.secondary ? (
                      <span className="ml-1.5 text-xs text-muted-foreground">
                        ({item.secondary})
                      </span>
                    ) : null}
                  </span>
                </Label>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function ShieldIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  )
}

function ServersIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="2" y="3" width="20" height="7" rx="1.5" />
      <rect x="2" y="14" width="20" height="7" rx="1.5" />
      <line x1="6" y1="6.5" x2="6.01" y2="6.5" />
      <line x1="6" y1="17.5" x2="6.01" y2="17.5" />
    </svg>
  )
}
