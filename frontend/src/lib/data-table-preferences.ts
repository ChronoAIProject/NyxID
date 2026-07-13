/**
 * Per-table column layout (order, freeze point, widths) persisted to
 * localStorage so a user's arrangement survives a reload.
 *
 * Everything read back is sanitized against the table's current columns: the
 * stored payload outlives the code that wrote it, so a column that has since
 * been renamed, added, or removed -- or a width from an older set of bounds,
 * or a hand-edited value -- must degrade to the default rather than render a
 * broken table.
 */

export interface DataTableColumnPreferences<Field extends string> {
  readonly order: readonly Field[];
  readonly frozenThrough: Field | null;
  readonly widths: Readonly<Record<Field, number>>;
}

export interface ColumnWidthBounds {
  readonly min: number;
  readonly max: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Reconcile a stored order with the columns that exist now: keep the stored
 * positions of the columns we still know and drop the ones we don't. A column
 * the stored payload never saw goes to the end -- the user has never placed it,
 * and dropping it into its default index would push it between two columns they
 * did place.
 */
function sanitizeOrder<Field extends string>(
  raw: unknown,
  defaults: readonly Field[],
): readonly Field[] {
  if (!Array.isArray(raw)) return defaults;
  const known = new Set<Field>(defaults);
  const order: Field[] = [];
  for (const field of raw) {
    if (typeof field !== "string") continue;
    const candidate = field as Field;
    if (!known.has(candidate) || order.includes(candidate)) continue;
    order.push(candidate);
  }
  if (order.length === 0) return defaults;
  return [...order, ...defaults.filter((field) => !order.includes(field))];
}

function sanitizeWidths<Field extends string>(
  raw: unknown,
  defaults: Readonly<Record<Field, number>>,
  bounds: ColumnWidthBounds,
): Readonly<Record<Field, number>> {
  if (!isRecord(raw)) return defaults;
  const widths: Record<string, number> = { ...defaults };
  for (const field of Object.keys(defaults)) {
    const width = raw[field];
    if (typeof width !== "number" || !Number.isFinite(width)) continue;
    if (width < bounds.min || width > bounds.max) continue;
    widths[field] = Math.round(width);
  }
  return widths as Record<Field, number>;
}

export function sanitizeColumnPreferences<Field extends string>(
  raw: unknown,
  defaults: DataTableColumnPreferences<Field>,
  bounds: ColumnWidthBounds,
): DataTableColumnPreferences<Field> {
  if (!isRecord(raw)) return defaults;
  const order = sanitizeOrder(raw.order, defaults.order);
  const frozenThrough =
    typeof raw.frozenThrough === "string" &&
    order.includes(raw.frozenThrough as Field)
      ? (raw.frozenThrough as Field)
      : null;
  return {
    order,
    frozenThrough,
    widths: sanitizeWidths(raw.widths, defaults.widths, bounds),
  };
}

export function loadColumnPreferences<Field extends string>(
  key: string,
  defaults: DataTableColumnPreferences<Field>,
  bounds: ColumnWidthBounds,
): DataTableColumnPreferences<Field> {
  try {
    const stored = window.localStorage.getItem(key);
    if (!stored) return defaults;
    return sanitizeColumnPreferences(JSON.parse(stored), defaults, bounds);
  } catch {
    // Corrupt JSON, or storage unavailable (private mode, blocked cookies).
    return defaults;
  }
}

export function saveColumnPreferences<Field extends string>(
  key: string,
  preferences: DataTableColumnPreferences<Field>,
  defaults: DataTableColumnPreferences<Field>,
): void {
  try {
    // Nothing to remember once the table is back to its defaults; clearing the
    // row also means a future change to the defaults reaches existing users.
    if (isDefaultColumnLayout(preferences, defaults)) {
      window.localStorage.removeItem(key);
      return;
    }
    window.localStorage.setItem(key, JSON.stringify(preferences));
  } catch {
    // A layout preference is never worth failing a render over.
  }
}

export function isDefaultColumnLayout<Field extends string>(
  preferences: DataTableColumnPreferences<Field>,
  defaults: DataTableColumnPreferences<Field>,
): boolean {
  return (
    preferences.frozenThrough === defaults.frozenThrough &&
    preferences.order.length === defaults.order.length &&
    preferences.order.every(
      (field, index) => field === defaults.order[index],
    ) &&
    Object.keys(defaults.widths).every(
      (field) =>
        preferences.widths[field as Field] === defaults.widths[field as Field],
    )
  );
}
