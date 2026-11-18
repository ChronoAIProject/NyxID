export type FormChange =
  | { readonly label: string; readonly before: string; readonly after: string; readonly added?: never; readonly removed?: never }
  | { readonly label: string; readonly added: readonly string[]; readonly removed: readonly string[]; readonly before?: never; readonly after?: never };

export interface ChangeField<Key extends PropertyKey = string> {
  readonly key: Key;
  readonly label: string;
  readonly format?: (value: unknown) => string;
  readonly list?: (value: unknown) => readonly string[];
}

export function changeValue(value: unknown): string {
  if (typeof value === "boolean") return value ? "On" : "Off";
  if (value === null || value === undefined || value === "") return "Empty";
  return String(value);
}

/** Only explicitly named fields enter a review. Never enumerate form values: they may hold secrets. */
export function describeChanges<T extends object>(before: object | undefined, after: T, fields: readonly ChangeField<keyof T>[]): FormChange[] {
  return fields.flatMap<FormChange>((field) => {
    const previous = (before as Record<PropertyKey, unknown> | undefined)?.[field.key];
    const current = after[field.key];
    if (field.list) {
      const oldList = field.list(previous), newList = field.list(current);
      const added = newList.filter((value) => !oldList.includes(value));
      const removed = oldList.filter((value) => !newList.includes(value));
      if (added.length || removed.length) return [{ label: field.label, added, removed }];
      if (oldList.join("\n") !== newList.join("\n")) return [{ label: field.label, before: oldList.join(", ") || "Empty", after: newList.join(", ") || "Empty" }];
      return [];
    }
    const format = field.format ?? changeValue;
    const oldText = format(previous), newText = format(current);
    return oldText === newText ? [] : [{ label: field.label, before: oldText, after: newText }];
  });
}
export const changeList = (value: unknown): string[] => Array.isArray(value) ? value.map(String) : [];
export const changeLines = (value: unknown): string[] => typeof value === "string" ? value.split("\n").map((line) => line.trim()).filter(Boolean) : [];
export const labelValue = (labels: Readonly<Record<string, string>>, empty = "Empty") => (value: unknown) => value === null || value === undefined || value === "" ? empty : labels[String(value)] ?? String(value);
export function recordChanges(before: Readonly<Record<string, unknown>> | undefined, after: Readonly<Record<string, unknown>>, label: (key: string) => string, format: (value: unknown) => string = changeValue): FormChange[] {
  return [...new Set([...Object.keys(before ?? {}), ...Object.keys(after)])].flatMap((key) => describeChanges({ value: before?.[key] }, { value: after[key] }, [{ key: "value", label: label(key), format }]));
}
