/**
 * Step 1 of the `nyxid service add` (ai-key) wizard: pick a service.
 *
 * Ports the catalog-grid UX from the old vanilla Mode A wizard
 * (`cli/src/wizard/assets/wizard.html:37-54` + `wizard.js:142-240`).
 * Renders every catalog entry as a card (title, description, and a
 * meta label like "paste API key" / "OAuth sign-in" / "device code")
 * under a "Simple setup" section, plus a single "Custom / self-hosted"
 * card under "Advanced" for services not in the catalog.
 *
 * Cards with a non-paste-key provider shape (oauth, device_code, ssh)
 * carry a small badge so the user knows what kind of flow they're
 * picking before they click.
 */

import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { ApiError, api } from "@/lib/api-client"
import { Input } from "@/components/ui/input"

interface CatalogEntry {
  readonly slug: string
  readonly name: string
  readonly description?: string
  readonly auth_method?: string
  readonly provider_type?: string
  readonly service_type?: string
  readonly requires_credential?: boolean
  readonly requires_gateway_url?: boolean
  readonly token_exchange_credential_fields?: readonly unknown[]
}

interface CatalogListResponse {
  readonly entries?: readonly CatalogEntry[]
  readonly services?: readonly CatalogEntry[]
}

export interface CatalogGridProps {
  readonly onSelect: (slug: string) => void
}

type FlowShape =
  | "no-auth"
  | "gateway-url"
  | "token-exchange"
  | "oauth"
  | "device-code"
  | "ssh"
  | "paste-key"

function flowShapeOf(entry: CatalogEntry): FlowShape {
  const pt = (entry.provider_type ?? "").toLowerCase()
  if ((entry.service_type ?? "http") === "ssh") return "ssh"
  if (pt === "oauth2") return "oauth"
  if (pt === "device_code") return "device-code"
  if (entry.requires_credential === false) return "no-auth"
  if (
    Array.isArray(entry.token_exchange_credential_fields) &&
    entry.token_exchange_credential_fields.length > 0
  ) {
    return "token-exchange"
  }
  if (entry.requires_gateway_url) return "gateway-url"
  return "paste-key"
}

function shapeLabel(shape: FlowShape, entry: CatalogEntry): string {
  switch (shape) {
    case "no-auth":
      return "1-click connect"
    case "gateway-url":
      return "URL + API key"
    case "token-exchange":
      return `${(entry.token_exchange_credential_fields ?? []).length} fields`
    case "oauth":
      return "OAuth sign-in"
    case "device-code":
      return "device code"
    case "ssh":
      return "SSH cert"
    case "paste-key":
      return "paste API key"
  }
}

const BADGE_LABEL: Partial<Record<FlowShape, string>> = {
  oauth: "OAuth",
  "device-code": "Device code",
  ssh: "SSH",
}

/**
 * Subsequence-based fuzzy match. Returns a numeric score (lower is
 * better) if every char of `query` appears in `target` in order, or
 * `null` otherwise. Bonuses for:
 *   - exact substring match (best score)
 *   - contiguous runs of matched chars (second best)
 *   - matches at word boundaries / after hyphens
 *
 * Good enough for a small catalog (< 100 entries); no worth pulling in
 * fuse.js for this.
 */
function fuzzyScore(target: string, query: string): number | null {
  if (!query) return 0
  const t = target.toLowerCase()
  const q = query.toLowerCase()
  // Exact substring wins. Lower score = earlier match = better.
  const sub = t.indexOf(q)
  if (sub >= 0) return sub
  // Subsequence: each char of q must appear in order in t.
  let ti = 0
  let qi = 0
  let score = 100 // baseline for non-contiguous
  let runLen = 0
  while (ti < t.length && qi < q.length) {
    if (t[ti] === q[qi]) {
      // Word-boundary bonus (start of string or after a hyphen / space).
      if (ti === 0 || t[ti - 1] === "-" || t[ti - 1] === " ") score -= 2
      // Contiguous-run bonus.
      runLen += 1
      score -= runLen
      qi += 1
    } else {
      runLen = 0
    }
    ti += 1
  }
  if (qi < q.length) return null
  return Math.max(score, 1)
}

export function CatalogGrid({ onSelect }: CatalogGridProps) {
  const [filter, setFilter] = useState("")
  const { data, isLoading, error } = useQuery({
    queryKey: ["cli-wizard", "catalog"],
    queryFn: async (): Promise<readonly CatalogEntry[]> => {
      const res = await api.get<CatalogListResponse>(
        "/catalog?include_all=true",
      )
      return res.entries ?? res.services ?? []
    },
  })

  const entries = data ?? []
  const f = filter.trim()
  // Fuzzy match against both slug and name; take the better score of
  // the two. Sort ascending (best first). No filter → preserve API
  // ordering.
  const visible = f
    ? entries
        .map((e) => {
          const slugScore = fuzzyScore(e.slug, f)
          const nameScore = fuzzyScore(e.name ?? "", f)
          const best =
            slugScore === null
              ? nameScore
              : nameScore === null
                ? slugScore
                : Math.min(slugScore, nameScore)
          return best === null ? null : ({ entry: e, score: best } as const)
        })
        .filter((x): x is { readonly entry: CatalogEntry; readonly score: number } =>
          x !== null,
        )
        .sort((a, b) => a.score - b.score)
        .map((x) => x.entry)
    : entries

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <label
          htmlFor="catalog-search"
          className="text-xs font-medium uppercase tracking-wide text-muted-foreground"
        >
          Search
        </label>
        <Input
          id="catalog-search"
          type="search"
          placeholder="search services…"
          autoComplete="off"
          spellCheck={false}
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value)
          }}
        />
      </div>

      <SectionLabel>Simple setup</SectionLabel>
      {isLoading ? (
        <p className="text-[12px] text-muted-foreground">Loading catalog…</p>
      ) : error ? (
        <p className="text-[12px] text-destructive">
          {error instanceof ApiError
            ? `Couldn't load the catalog: ${error.message} (${String(error.status)})`
            : "Couldn't load the catalog. Check the CLI logs for details."}
        </p>
      ) : visible.length === 0 ? (
        <p className="text-[12px] text-muted-foreground">
          {f ? "No services match your search." : "Catalog is empty."}
        </p>
      ) : (
        // Cap the grid at ~3 rows (6 cards on 2-col layouts) and let
        // the rest scroll within. Card min-height is 132px + 12px gap,
        // so 3 rows = (132 × 3) + (12 × 2) = 420px. Keeps the Advanced
        // section + Custom card visible without scrolling the whole
        // page.
        <div
          className="max-h-[420px] overflow-y-auto overscroll-contain pr-1"
          role="list"
        >
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {visible.map((entry) => (
              <CatalogCard
                key={entry.slug}
                entry={entry}
                onClick={() => {
                  onSelect(entry.slug)
                }}
              />
            ))}
          </div>
        </div>
      )}

      <SectionLabel>Advanced</SectionLabel>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <CustomCard
          onClick={() => {
            onSelect("__custom__")
          }}
        />
      </div>
    </div>
  )
}

function SectionLabel({ children }: { readonly children: React.ReactNode }) {
  return (
    <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
      {children}
    </div>
  )
}

function CatalogCard({
  entry,
  onClick,
}: {
  readonly entry: CatalogEntry
  readonly onClick: () => void
}) {
  const shape = flowShapeOf(entry)
  const badge = BADGE_LABEL[shape]
  return (
    <button
      type="button"
      onClick={onClick}
      role="listitem"
      className="group relative flex min-h-[132px] flex-col items-start gap-1 rounded-xl border border-border/50 bg-card/60 p-4 text-left transition-colors duration-300 hover:border-white/[0.15] hover:bg-card focus-visible:outline-none"
    >
      {badge ? (
        <span className="absolute right-3 top-3 rounded-full border border-border bg-muted/60 px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
          {badge}
        </span>
      ) : null}
      <span className="text-[13px] font-semibold text-foreground">
        {entry.name || entry.slug}
      </span>
      {entry.description ? (
        <span className="line-clamp-2 text-xs text-muted-foreground">
          {entry.description}
        </span>
      ) : null}
      <span className="mt-auto text-[11px] text-text-tertiary">
        {shapeLabel(shape, entry)}
      </span>
    </button>
  )
}

function CustomCard({ onClick }: { readonly onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex min-h-[132px] flex-col items-start gap-1 rounded-xl border border-dashed border-border/50 bg-transparent p-4 text-left transition-colors duration-300 hover:border-white/[0.15] hover:bg-card/40 focus-visible:outline-none"
    >
      <span className="text-[13px] font-semibold text-foreground">
        Custom / self-hosted…
      </span>
      <span className="text-xs text-muted-foreground">
        For anything that isn't in the catalog above — paste your own
        endpoint URL + credential.
      </span>
    </button>
  )
}
