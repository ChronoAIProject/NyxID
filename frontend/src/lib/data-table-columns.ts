/**
 * Column layout for resizable, freezable data tables.
 *
 * Widths are held in one CSS variable per column, set on the table element.
 * Column widths, the table's min-width, and the sticky offsets of frozen
 * columns are all expressed against those variables, so resizing a column means
 * rewriting a single variable and letting the browser reflow the rest -- no
 * re-render of every row on each pointer move.
 */

export function columnWidthVar(field: string): string {
  return `--col-w-${field}`;
}

/** CSS length summing the given columns' width variables, in order. */
export function sumOfColumnWidths(fields: readonly string[]): string {
  return `calc(${fields.map((field) => `var(${columnWidthVar(field)})`).join(" + ")})`;
}

export function columnWidthVars<Field extends string>(
  order: readonly Field[],
  widths: Readonly<Record<Field, number>>,
): Record<string, string> {
  return Object.fromEntries(
    order.map((field) => [columnWidthVar(field), `${String(widths[field])}px`]),
  );
}

export function clampColumnWidth(
  width: number,
  min: number,
  max: number,
): number {
  return Math.round(Math.min(max, Math.max(min, width)));
}

/** The columns frozen when freezing runs through `frozenThrough`, in order. */
export function frozenColumnFields<Field extends string>(
  order: readonly Field[],
  frozenThrough: Field | null,
): readonly Field[] {
  if (!frozenThrough) return [];
  const last = order.indexOf(frozenThrough);
  return last < 0 ? [] : order.slice(0, last + 1);
}

/**
 * Sticky `left` for a frozen column: the running sum of the widths of the
 * frozen columns before it. Undefined when the column is not frozen.
 */
export function stickyColumnLeft<Field extends string>(
  frozenFields: readonly Field[],
  field: Field,
): string | undefined {
  const index = frozenFields.indexOf(field);
  if (index < 0) return undefined;
  if (index === 0) return "0px";
  return sumOfColumnWidths(frozenFields.slice(0, index));
}
