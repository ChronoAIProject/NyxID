import { CrmPersonName } from "@/components/crm/people-picker";
import { currencyDecimal, formatCrmMoney } from "@/lib/crm/money";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import type { CrmField, CrmFilterOperator, CrmRecordLink } from "@/schemas/crm";

const TEXT_OPERATORS = [
  "eq",
  "neq",
  "in",
  "contains",
  "is_set",
  "is_unset",
] as const satisfies readonly CrmFilterOperator[];
const ORDERED_OPERATORS = [
  "eq",
  "neq",
  "gt",
  "gte",
  "lt",
  "lte",
  "is_set",
  "is_unset",
] as const satisfies readonly CrmFilterOperator[];
const BOOLEAN_OPERATORS = [
  "eq",
  "is_set",
  "is_unset",
] as const satisfies readonly CrmFilterOperator[];
const ENUM_OPERATORS = [
  "eq",
  "neq",
  "in",
  "nin",
  "is_set",
  "is_unset",
] as const satisfies readonly CrmFilterOperator[];
const RECORD_OPERATORS = [
  "eq",
  "in",
  "is_set",
  "is_unset",
] as const satisfies readonly CrmFilterOperator[];

export const CRM_FILTER_OPERATOR_LABELS: Readonly<
  Record<CrmFilterOperator, string>
> = {
  eq: "is",
  neq: "is not",
  in: "is any of",
  nin: "is none of",
  contains: "contains",
  gt: "is greater than",
  gte: "is at least",
  lt: "is less than",
  lte: "is at most",
  is_set: "is set",
  is_unset: "is not set",
};

export interface CrmCellValue {
  readonly value: unknown;
  readonly links?: readonly CrmRecordLink[];
}

export interface CrmFieldKindDefinition {
  readonly filterOperators: readonly CrmFilterOperator[];
  readonly formatInput: (field: CrmField, value: unknown) => unknown;
  readonly parseInput: (field: CrmField, value: unknown) => unknown;
  readonly renderCell: (field: CrmField, cell: CrmCellValue) => ReactNode;
}

function empty(value: unknown): boolean {
  return value === null || value === undefined || value === "";
}

function text(value: unknown): string {
  return empty(value) ? "" : String(value);
}

function finiteNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value !== "string" || !value.trim()) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function parseNumberInput(value: unknown): number | null {
  if (empty(value) || (typeof value === "string" && !value.trim())) return null;
  const parsed = finiteNumber(value);
  if (parsed === null) throw new Error("Enter a finite number.");
  return parsed;
}

function currencyMinor(value: unknown): number | null {
  if (typeof value === "number" && Number.isSafeInteger(value)) return value;
  if (
    typeof value === "object" &&
    value !== null &&
    "amount_minor" in value &&
    typeof value.amount_minor === "number" &&
    Number.isSafeInteger(value.amount_minor)
  ) {
    return value.amount_minor;
  }
  return null;
}

function parseCurrencyInput(field: CrmField, value: unknown): number | null {
  if (empty(value)) return null;
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value)) {
      throw new Error("Enter an integer amount in minor units.");
    }
    return value;
  }
  if (typeof value === "object" && value !== null) {
    const minor = currencyMinor(value);
    if (minor === null)
      throw new Error("Enter an integer amount in minor units.");
    if (
      field.type === "currency" &&
      "currency" in value &&
      value.currency !== field.currency
    ) {
      throw new Error(`Currency must be ${field.currency}.`);
    }
    return minor;
  }
  const input = String(value).trim();
  const match = /^(-?)(\d+)(?:\.(\d{0,2}))?$/.exec(input);
  if (!match?.[2])
    throw new Error("Enter an amount with at most 2 decimal places.");
  const whole = BigInt(match[2]);
  const minor = BigInt((match[3] ?? "").padEnd(2, "0"));
  const amount = whole * 100n + minor;
  const signed = match[1] === "-" ? -amount : amount;
  if (
    signed > BigInt(Number.MAX_SAFE_INTEGER) ||
    signed < -BigInt(Number.MAX_SAFE_INTEGER)
  )
    throw new Error("This amount is too large.");
  return Number(signed);
}

function formatCurrencyInput(value: unknown): string {
  const minor = currencyMinor(value);
  if (minor === null) return "";
  return currencyDecimal(minor);
}

function parsePercentInput(value: unknown): number | null {
  const parsed = parseNumberInput(value);
  return parsed === null ? null : parsed / 100;
}

function formatPercentInput(value: unknown): string {
  const parsed = finiteNumber(value);
  return parsed === null ? "" : String(parsed * 100);
}

function isCalendarDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match?.[1] || !match[2] || !match[3]) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(Date.UTC(year, month - 1, day));
  return (
    date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
  );
}

function parseDateInput(value: unknown): string | null {
  if (empty(value)) return null;
  const parsed = String(value);
  if (!isCalendarDate(parsed)) throw new Error("Enter a valid calendar date.");
  return parsed;
}

function formatDateInput(value: unknown): string {
  return typeof value === "string" && isCalendarDate(value) ? value : "";
}

function parseDatetimeInput(value: unknown): string | null {
  if (empty(value)) return null;
  const date = new Date(String(value));
  if (Number.isNaN(date.getTime()))
    throw new Error("Enter a valid date and time.");
  return date.toISOString();
}

function formatDatetimeInput(value: unknown): string {
  if (typeof value !== "string" || !value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function plainCell(value: unknown): ReactNode {
  return empty(value) ? (
    <span className="text-text-tertiary">—</span>
  ) : (
    String(value)
  );
}

const textDefinition: CrmFieldKindDefinition = {
  filterOperators: TEXT_OPERATORS,
  formatInput: (_field, value) => text(value),
  parseInput: (_field, value) => (empty(value) ? null : String(value)),
  renderCell: (_field, { value }) => plainCell(value),
};

const numberDefinition: CrmFieldKindDefinition = {
  filterOperators: ORDERED_OPERATORS,
  formatInput: (field, value) =>
    field.type === "number" && field.format === "percent"
      ? formatPercentInput(value)
      : (finiteNumber(value)?.toString() ?? ""),
  parseInput: (field, value) =>
    field.type === "number" && field.format === "percent"
      ? parsePercentInput(value)
      : parseNumberInput(value),
  renderCell: (field, { value }) => {
    const parsed = finiteNumber(value);
    if (parsed === null) return plainCell(null);
    return field.type === "number" && field.format === "percent"
      ? new Intl.NumberFormat(undefined, { style: "percent" }).format(parsed)
      : new Intl.NumberFormat().format(parsed);
  },
};

const currencyDefinition: CrmFieldKindDefinition = {
  filterOperators: ORDERED_OPERATORS,
  formatInput: (_field, value) => formatCurrencyInput(value),
  parseInput: (field, value) => parseCurrencyInput(field, value),
  renderCell: (field, { value }) => {
    const minor = currencyMinor(value);
    if (minor === null || field.type !== "currency") return plainCell(null);
    return formatCrmMoney(minor, field.currency);
  },
};

const booleanDefinition: CrmFieldKindDefinition = {
  filterOperators: BOOLEAN_OPERATORS,
  formatInput: (field, value) =>
    empty(value) ? (field.required ? false : null) : value === true,
  parseInput: (_field, value) =>
    empty(value) ? null : value === true || value === "true",
  renderCell: (_field, { value }) =>
    empty(value) ? plainCell(null) : value === true ? "Yes" : "No",
};

const dateDefinition: CrmFieldKindDefinition = {
  filterOperators: ORDERED_OPERATORS,
  formatInput: (_field, value) => formatDateInput(value),
  parseInput: (_field, value) => parseDateInput(value),
  renderCell: (_field, { value }) => plainCell(formatDateInput(value)),
};

const datetimeDefinition: CrmFieldKindDefinition = {
  filterOperators: ORDERED_OPERATORS,
  formatInput: (_field, value) => formatDatetimeInput(value),
  parseInput: (_field, value) => parseDatetimeInput(value),
  renderCell: (_field, { value }) => {
    if (typeof value !== "string") return plainCell(null);
    const parsed = new Date(value);
    return Number.isNaN(parsed.getTime())
      ? plainCell(null)
      : parsed.toLocaleString();
  },
};

const selectDefinition: CrmFieldKindDefinition = {
  filterOperators: ENUM_OPERATORS,
  formatInput: (_field, value) => text(value),
  parseInput: (_field, value) => (empty(value) ? null : String(value)),
  renderCell: (field, { value }) => {
    if (empty(value) || field.type !== "select") return plainCell(null);
    const option = field.options.find((candidate) => candidate.value === value);
    return (
      <Badge variant="secondary">
        {option?.label ?? String(value)}
        {option?.archived ? " (archived)" : ""}
      </Badge>
    );
  },
};

const recordDefinition: CrmFieldKindDefinition = {
  filterOperators: RECORD_OPERATORS,
  formatInput: (_field, value) => value,
  parseInput: (_field, value) => value,
  renderCell: (_field, { links }) =>
    links?.length
      ? links
          .map((link) =>
            link.archived && !link.label.includes("(archived)")
              ? `${link.label} (archived)`
              : link.label,
          )
          .join(", ")
      : plainCell(null),
};

const userDefinition: CrmFieldKindDefinition = {
  filterOperators: ENUM_OPERATORS,
  formatInput: (_field, value) => text(value),
  parseInput: (_field, value) => (empty(value) ? null : String(value)),
  renderCell: (_field, { value }) => (
    <CrmPersonName id={typeof value === "string" ? value : undefined} />
  ),
};

export const CRM_FIELD_KINDS: Readonly<
  Record<CrmField["type"], CrmFieldKindDefinition>
> = {
  text: textDefinition,
  number: numberDefinition,
  currency: currencyDefinition,
  boolean: booleanDefinition,
  date: dateDefinition,
  datetime: datetimeDefinition,
  select: selectDefinition,
  record: recordDefinition,
  user: userDefinition,
};

export function crmFieldKind(field: CrmField): CrmFieldKindDefinition {
  return CRM_FIELD_KINDS[field.type];
}

export function crmFilterOperators(field: CrmField) {
  return crmFieldKind(field).filterOperators;
}

export function isCrmFilterOperatorAllowed(
  field: CrmField,
  operator: CrmFilterOperator,
): boolean {
  return crmFilterOperators(field).includes(operator);
}

export function isCrmSortableField(field: CrmField): boolean {
  return field.type !== "record" && field.type !== "user";
}

export function formatCrmFieldInput(field: CrmField, value: unknown): unknown {
  return crmFieldKind(field).formatInput(field, value);
}

export function parseCrmFieldInput(field: CrmField, value: unknown): unknown {
  return crmFieldKind(field).parseInput(field, value);
}

export function renderCrmCell(
  field: CrmField,
  value: unknown,
  links?: readonly CrmRecordLink[],
): ReactNode {
  return crmFieldKind(field).renderCell(field, { value, links });
}
