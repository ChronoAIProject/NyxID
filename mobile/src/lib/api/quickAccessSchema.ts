import { z } from "zod";

/*
 * The Duo's quick-access rail, saved to the account
 * (`GET/PUT/DELETE /users/me/quick-access`). A fixed destination from the
 * side menu, or a pinned agent (its UUID) or crew (its `nyxg-` id).
 */
export const QUICK_ACCESS_DESTINATIONS = [
  "new_chat",
  "notifications",
  "scan_qr",
  "terminal_login_code",
  "recent_chats",
  "automations",
  "machines",
  "plugins",
  "activity",
  "create_agent",
  "settings",
  "agents",
  "crews",
] as const;
export type QuickAccessDestination = (typeof QUICK_ACCESS_DESTINATIONS)[number];

/** What everyone sees until they customise (and after a reset). */
export const QUICK_ACCESS_DEFAULT: readonly QuickAccessDestination[] = [
  "new_chat",
  "notifications",
  "scan_qr",
  "recent_chats",
  "settings",
];

/** The account holds at most this many entries (the server enforces it). */
export const QUICK_ACCESS_MAX_ITEMS = 12;

export const quickAccessItemSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("destination"),
    id: z.enum(QUICK_ACCESS_DESTINATIONS),
  }),
  // Any UUID form the server accepts (it canonicalises to lowercase).
  z.object({
    kind: z.literal("agent"),
    id: z
      .string()
      .regex(/^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/),
  }),
  z.object({
    kind: z.literal("crew"),
    id: z.string().regex(/^nyxg-[a-fA-F0-9]{32}$/),
  }),
]);
export type QuickAccessItem = z.infer<typeof quickAccessItemSchema>;

export type QuickAccessState = {
  items: QuickAccessItem[];
  /** False while the person still has the default list. */
  customized: boolean;
  /** Some saved entries need a newer client/server; only Reset may replace them. */
  hasUnsupportedEntries?: boolean;
};

export const QUICK_ACCESS_DEFAULT_STATE: QuickAccessState = {
  items: QUICK_ACCESS_DEFAULT.map((id) => ({ kind: "destination", id })),
  customized: false,
};

/**
 * The server's answer. Entries this app does not know (a destination or kind
 * added by a newer server) are skipped instead of failing the whole list.
 */
export const quickAccessResponseSchema = z
  .object({ items: z.array(z.unknown()), customized: z.boolean(), has_unsupported_entries: z.boolean().optional() })
  .transform(({ items, customized, has_unsupported_entries }) => {
    const known = items.flatMap((item) => {
      const parsed = quickAccessItemSchema.safeParse(item);
      return parsed.success ? [parsed.data] : [];
    });
    return { customized, items: known,
      ...(has_unsupported_entries || known.length !== items.length ? { hasUnsupportedEntries: true } : {}) };
  });

/** Two entries name the same thing (the server rejects duplicates). */
export function sameQuickAccessItem(a: QuickAccessItem, b: QuickAccessItem) {
  return a.kind === b.kind && a.id.toLowerCase() === b.id.toLowerCase();
}
