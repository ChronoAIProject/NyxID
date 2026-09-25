export function sameValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (
    a === null ||
    b === null ||
    typeof a !== "object" ||
    typeof b !== "object"
  )
    return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const left = Object.entries(a);
  const right = Object.entries(b);
  return (
    left.length === right.length &&
    left.every(
      ([key, value]) =>
        Object.hasOwn(b, key) &&
        sameValue(value, (b as Record<string, unknown>)[key]),
    )
  );
}

/** Arrays and nested objects are replacement fields in the admin update APIs. */
export function changedFields<T extends object>(
  before: T,
  after: T,
): Partial<T> {
  return Object.fromEntries(
    Object.entries(after).filter(
      ([key, value]) =>
        value !== undefined &&
        !sameValue((before as Record<string, unknown>)[key], value),
    ),
  ) as Partial<T>;
}

export interface FormChange {
  readonly field: string;
  readonly before: string;
  readonly after: string;
}

function displayValue(value: unknown): string {
  if (value === null || value === undefined || value === "") return "Not set";
  if (typeof value === "boolean") return value ? "Enabled" : "Disabled";
  return typeof value === "object"
    ? JSON.stringify(value, null, 2)
    : String(value);
}

export function describeChanges(
  before: object,
  patch: object,
  options: {
    labels?: Record<string, string>;
    secretFields?: readonly string[];
  } = {},
): FormChange[] {
  return Object.entries(patch).map(([key, value]) => {
    const secret = options.secretFields?.includes(key);
    return {
      field:
        options.labels?.[key] ??
        key.replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase()),
      before: secret
        ? "Stored value hidden"
        : displayValue((before as Record<string, unknown>)[key]),
      after: secret
        ? value === null ||
          value === "" ||
          (Array.isArray(value) && value.length === 0)
          ? "Clear stored value"
          : "Replace stored value"
        : displayValue(value),
    };
  });
}

/** Compare only values that a sparse write would replace. */
export function hasFieldConflicts(
  before: object,
  current: object,
  patch: object,
): boolean {
  return Object.keys(patch).some(
    (key) =>
      !sameValue(
        (before as Record<string, unknown>)[key],
        (current as Record<string, unknown>)[key],
      ),
  );
}

export function normalizedSet(values: readonly string[]): string[] {
  return [
    ...new Set(values.map((value) => value.trim()).filter(Boolean)),
  ].sort();
}
