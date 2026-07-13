import type {
  AdminAuditLogActorFilter,
  AdminAuditLogFilterField,
  AdminAuditLogFilterKey,
  AdminAuditLogFilterOptions,
  AdminAuditLogSearchField,
  AdminAuditLogSearchFieldKey,
  AdminAuditLogSearchFilter,
  AdminAuditLogSearchState,
  AdminAuditLogSort,
  AdminAuditLogStatusFilter,
} from "@/types/admin";
import type { AppliedDataTableFilter } from "@/types/data-table";

const STATUS_FILTERS = ["2xx", "3xx", "4xx", "5xx", "none"] as const;
const ACTOR_FILTERS = ["user", "agent", "anonymous"] as const;

const MAX_MULTI_FILTER_VALUES = 32;
const MAX_SEARCH_GROUPS = 5;
/** Must stay >= the number of custom-text filters, or filling them all fails. */
const MAX_CUSTOM_FILTER_GROUPS = 8;
const MAX_SEARCH_VALUES_PER_GROUP = 8;
const MAX_SEARCH_VALUES = 32;
const MAX_SEARCH_VALUE_LENGTH = 256;

const SORTS = [
  "-created_at",
  "created_at",
  "event_type",
  "-event_type",
  "api_key_name",
  "-api_key_name",
  "api_key_id",
  "-api_key_id",
  "user_id",
  "-user_id",
  "ip_address",
  "-ip_address",
  "user_agent",
  "-user_agent",
  "status",
  "-status",
] as const satisfies readonly AdminAuditLogSort[];

export const ADMIN_AUDIT_LOG_SORTS: readonly AdminAuditLogSort[] = SORTS;

/** Presentation order in the filter popover; mirrors the table's columns. */
const FILTER_KEYS = [
  "event_type",
  "status",
  "actor",
  "created_at",
  "api_key_name",
  "user_id",
  "api_key_id",
  "ip_address",
  "user_agent",
] as const;

/**
 * Columns whose values are unbounded (UUIDs, IPs, User-Agent strings), so the
 * filter offers a free-text `contains` box instead of checkbox options.
 */
const TEXT_FILTER_KEYS = [
  "api_key_name",
  "user_id",
  "api_key_id",
  "ip_address",
  "user_agent",
] as const satisfies readonly AdminAuditLogFilterKey[];

const TEXT_FILTER_LABELS: Record<(typeof TEXT_FILTER_KEYS)[number], string> = {
  api_key_name: "Agent",
  user_id: "User ID",
  api_key_id: "API Key ID",
  ip_address: "IP address",
  user_agent: "User agent",
};

function isTextFilterKey(value: unknown): value is AdminAuditLogFilterKey {
  return (TEXT_FILTER_KEYS as readonly string[]).includes(value as string);
}

export const ADMIN_AUDIT_LOG_SEARCH_FIELDS = [
  { key: "event_type", label: "Event type" },
  { key: "user_id", label: "User ID" },
  { key: "api_key", label: "Agent / API key" },
  { key: "ip_address", label: "IP address" },
  { key: "user_agent", label: "User agent" },
] as const satisfies readonly AdminAuditLogSearchField[];

const SEARCH_FIELD_KEYS = ADMIN_AUDIT_LOG_SEARCH_FIELDS.map(({ key }) => key);

const STATUS_LABELS: Record<AdminAuditLogStatusFilter, string> = {
  "2xx": "2xx Success",
  "3xx": "3xx Redirect",
  "4xx": "4xx Client error",
  "5xx": "5xx Server error",
  none: "No status",
};

const ACTOR_LABELS: Record<AdminAuditLogActorFilter, string> = {
  user: "User session",
  agent: "Agent API key",
  anonymous: "Anonymous",
};

function option(value: string, label: string) {
  return { value, label } as const;
}

/**
 * Filters we can describe without the server.
 *
 * `event_type` is deliberately absent: its vocabulary is open and discovered
 * from the data, so it only appears once the server advertises its options.
 */
function fallbackFilterFields(): readonly AdminAuditLogFilterField[] {
  return [
    {
      key: "status",
      label: "Status",
      value_type: "enum",
      operator: "is",
      multiple: true,
      options: STATUS_FILTERS.map((value) => option(value, STATUS_LABELS[value])),
    },
    {
      key: "actor",
      label: "Actor",
      value_type: "enum",
      operator: "is",
      multiple: true,
      options: ACTOR_FILTERS.map((value) => option(value, ACTOR_LABELS[value])),
    },
    ...TEXT_FILTER_KEYS.map((key) => textFilterField(key)),
  ];
}

function textFilterField(
  key: (typeof TEXT_FILTER_KEYS)[number],
): AdminAuditLogFilterField {
  return {
    key,
    label: TEXT_FILTER_LABELS[key],
    value_type: "text",
    operator: "contains",
    multiple: false,
    options: [],
    supports_custom_text: true,
  };
}

function isFilterKey(value: unknown): value is AdminAuditLogFilterKey {
  return (
    typeof value === "string" &&
    FILTER_KEYS.includes(value as AdminAuditLogFilterKey)
  );
}

/**
 * Uses the server-described fields when available and fills any missing field
 * from the statically-known options during rolling deployments.
 */
export function getAdminAuditLogFilterFields(
  filterOptions?: AdminAuditLogFilterOptions,
): readonly AdminAuditLogFilterField[] {
  const fallbacks = fallbackFilterFields();
  const serverFields = filterOptions?.fields ?? [];
  const seen = new Set<AdminAuditLogFilterKey>();
  const fields: AdminAuditLogFilterField[] = [];

  for (const field of serverFields) {
    const isDateField =
      field.key === "created_at" &&
      field.value_type === "date" &&
      field.operator === "between" &&
      Array.isArray(field.options) &&
      field.options.length === 0;
    // A text filter is useless without the custom-text box, so it only survives
    // when the server also advertises that it accepts free text.
    const isTextField =
      isTextFilterKey(field.key) &&
      field.value_type === "text" &&
      field.operator === "contains" &&
      field.supports_custom_text === true;
    const isOptionField =
      field.key !== "created_at" &&
      field.value_type === "enum" &&
      field.operator === "is" &&
      Array.isArray(field.options) &&
      field.options.length > 0;

    if (
      !isFilterKey(field.key) ||
      seen.has(field.key) ||
      (!isDateField && !isTextField && !isOptionField)
    ) {
      continue;
    }

    const options = field.options.filter(
      (item) =>
        typeof item.value === "string" &&
        item.value !== "" &&
        typeof item.label === "string" &&
        item.label !== "",
    );
    if (!isDateField && !isTextField && options.length === 0) continue;

    // Only offer a text box when the server says the filter takes custom text
    // AND we know how to encode it: an older server in a rolling deploy sends no
    // flag and would reject `custom_filters` outright.
    const supportsCustomText =
      field.supports_custom_text === true && isCustomTextFilterKey(field.key);

    seen.add(field.key);
    fields.push(
      isDateField
        ? {
            ...field,
            options,
            date_modes: ["dates", "range"],
            max_values: MAX_MULTI_FILTER_VALUES,
          }
        : { ...field, options, supports_custom_text: supportsCustomText },
    );
  }

  for (const fallback of fallbacks) {
    if (!seen.has(fallback.key)) fields.push(fallback);
  }

  // Keep a stable presentation order regardless of what the server sent.
  return FILTER_KEYS.flatMap((key) => {
    const field = fields.find((item) => item.key === key);
    return field ? [field] : [];
  });
}

function isSearchFieldKey(
  value: unknown,
): value is AdminAuditLogSearchFieldKey {
  return (
    typeof value === "string" &&
    SEARCH_FIELD_KEYS.includes(value as AdminAuditLogSearchFieldKey)
  );
}

/**
 * Returns server-advertised search fields in stable table order. Scoped search
 * stays hidden for older backends that do not advertise support.
 */
export function getAdminAuditLogSearchFields(
  filterOptions?: AdminAuditLogFilterOptions,
): readonly AdminAuditLogSearchField[] {
  if (filterOptions?.search_fields === undefined) {
    return [];
  }

  const fieldsByKey = new Map<
    AdminAuditLogSearchFieldKey,
    AdminAuditLogSearchField
  >();
  for (const field of filterOptions.search_fields) {
    if (
      isSearchFieldKey(field.key) &&
      typeof field.label === "string" &&
      field.label.trim() !== "" &&
      !fieldsByKey.has(field.key)
    ) {
      fieldsByKey.set(field.key, { key: field.key, label: field.label.trim() });
    }
  }

  return SEARCH_FIELD_KEYS.flatMap((key) => {
    const field = fieldsByKey.get(key);
    return field ? [field] : [];
  });
}

function normalizeSearchValues(value: unknown): readonly string[] | undefined {
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.length > MAX_SEARCH_VALUES_PER_GROUP
  ) {
    return undefined;
  }

  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const rawTerm of value) {
    if (typeof rawTerm !== "string") return undefined;
    const term = rawTerm.trim();
    if (term === "" || [...term].length > MAX_SEARCH_VALUE_LENGTH) {
      return undefined;
    }
    const identity = term.toLowerCase();
    if (!seen.has(identity)) {
      seen.add(identity);
      normalized.push(term);
    }
  }

  return normalized;
}

function normalizeSearchFilterGroups(
  value: unknown,
): readonly AdminAuditLogSearchFilter[] | undefined {
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.length > MAX_SEARCH_GROUPS
  ) {
    return undefined;
  }

  const fields = new Set<AdminAuditLogSearchFieldKey>();
  const groups: AdminAuditLogSearchFilter[] = [];
  let totalValues = 0;

  for (const rawGroup of value) {
    if (
      typeof rawGroup !== "object" ||
      rawGroup === null ||
      Array.isArray(rawGroup)
    ) {
      return undefined;
    }
    const group = rawGroup as Record<string, unknown>;
    if (
      Object.keys(group).some((key) => key !== "field" && key !== "values") ||
      !isSearchFieldKey(group.field) ||
      fields.has(group.field)
    ) {
      return undefined;
    }
    const values = normalizeSearchValues(group.values);
    if (!values) return undefined;

    totalValues += values.length;
    if (totalValues > MAX_SEARCH_VALUES) return undefined;
    fields.add(group.field);
    groups.push({ field: group.field, values });
  }

  return groups.sort(
    (left, right) =>
      SEARCH_FIELD_KEYS.indexOf(left.field) -
      SEARCH_FIELD_KEYS.indexOf(right.field),
  );
}

/** Parses and canonicalizes field-scoped search URL state. */
export function parseAdminAuditLogSearchFilters(
  raw: unknown,
): readonly AdminAuditLogSearchFilter[] | undefined {
  if (Array.isArray(raw)) return normalizeSearchFilterGroups(raw);
  if (typeof raw !== "string" || raw === "") return undefined;
  try {
    return normalizeSearchFilterGroups(JSON.parse(raw));
  } catch {
    return undefined;
  }
}

/** Encodes validated field groups as the canonical API/URL JSON value. */
export function encodeAdminAuditLogSearchFilters(
  filters: readonly AdminAuditLogSearchFilter[],
): string | undefined {
  const normalized = normalizeSearchFilterGroups(filters);
  return normalized ? JSON.stringify(normalized) : undefined;
}

export function getAdminAuditLogSearchFilters(
  search: AdminAuditLogSearchState,
): readonly AdminAuditLogSearchFilter[] {
  return parseAdminAuditLogSearchFilters(search.search_filters) ?? [];
}

/** Upserts or removes one field group and returns canonical URL state. */
export function updateAdminAuditLogSearchFilters(
  current: unknown,
  field: AdminAuditLogSearchFieldKey,
  values: readonly string[] | undefined,
): string | undefined {
  const filters = parseAdminAuditLogSearchFilters(current) ?? [];
  const remaining = filters.filter((filter) => filter.field !== field);
  if (values === undefined || values.length === 0) {
    return remaining.length > 0
      ? encodeAdminAuditLogSearchFilters(remaining)
      : undefined;
  }
  return encodeAdminAuditLogSearchFilters([...remaining, { field, values }]);
}

/**
 * Filters that accept free text, and stay in lockstep with the backend's
 * `ADMIN_CUSTOM_TEXT_FILTERS`. A filter can only take custom text when it maps
 * to a single string column the server can run a `contains` against, which rules
 * out `status` and `actor` (both derived) and `created_at` (a date).
 */
export const ADMIN_AUDIT_LOG_CUSTOM_TEXT_FILTERS = [
  "event_type",
  "api_key_name",
  "user_id",
  "api_key_id",
  "ip_address",
  "user_agent",
] as const satisfies readonly AdminAuditLogFilterKey[];

export type AdminAuditLogCustomFilters = Partial<
  Record<AdminAuditLogFilterKey, readonly string[]>
>;

function isCustomTextFilterKey(value: unknown): value is AdminAuditLogFilterKey {
  return (ADMIN_AUDIT_LOG_CUSTOM_TEXT_FILTERS as readonly string[]).includes(
    value as string,
  );
}

function normalizeCustomFilters(
  value: unknown,
): AdminAuditLogCustomFilters | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined;
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0 || entries.length > MAX_CUSTOM_FILTER_GROUPS) {
    return undefined;
  }

  const normalized: Record<string, readonly string[]> = {};
  let totalValues = 0;

  // Emit keys in a fixed order so the same selection always encodes to the same
  // URL, whatever order the user filled the filters in.
  for (const key of ADMIN_AUDIT_LOG_CUSTOM_TEXT_FILTERS) {
    const raw = entries.find(([entryKey]) => entryKey === key)?.[1];
    if (raw === undefined) continue;
    const values = normalizeSearchValues(raw);
    if (!values) return undefined;
    totalValues += values.length;
    if (totalValues > MAX_SEARCH_VALUES) return undefined;
    normalized[key] = values;
  }

  if (entries.some(([key]) => !isCustomTextFilterKey(key))) return undefined;
  if (Object.keys(normalized).length === 0) return undefined;
  return normalized;
}

/** Parses and canonicalizes the custom-text URL state. */
export function parseAdminAuditLogCustomFilters(
  raw: unknown,
): AdminAuditLogCustomFilters | undefined {
  if (typeof raw === "object" && raw !== null) {
    return normalizeCustomFilters(raw);
  }
  if (typeof raw !== "string" || raw === "") return undefined;
  try {
    return normalizeCustomFilters(JSON.parse(raw));
  } catch {
    return undefined;
  }
}

/** Encodes validated custom text as the canonical API/URL JSON value. */
export function encodeAdminAuditLogCustomFilters(
  filters: AdminAuditLogCustomFilters,
): string | undefined {
  const normalized = normalizeCustomFilters(filters);
  return normalized ? JSON.stringify(normalized) : undefined;
}

export function getAdminAuditLogCustomValues(
  search: AdminAuditLogSearchState,
  key: AdminAuditLogFilterKey,
): readonly string[] {
  return parseAdminAuditLogCustomFilters(search.custom_filters)?.[key] ?? [];
}

/**
 * Replaces every filter's custom text and returns canonical URL state.
 *
 * The UI hands back a draft keyed by every filter it rendered, so prune the ones
 * that carry no text (and any filter that takes none) before encoding: the URL
 * form is strict and would reject the padded shape wholesale.
 */
export function adminAuditLogCustomFilterPatch(
  filters: AdminAuditLogCustomFilters,
): Partial<AdminAuditLogSearchState> {
  const populated: AdminAuditLogCustomFilters = {};
  for (const key of ADMIN_AUDIT_LOG_CUSTOM_TEXT_FILTERS) {
    const values = filters[key];
    if (values && values.length > 0) populated[key] = values;
  }
  return { custom_filters: encodeAdminAuditLogCustomFilters(populated) };
}

function getAdminAuditLogFilterCsv(
  search: AdminAuditLogSearchState,
  key: AdminAuditLogFilterKey,
): string | undefined {
  switch (key) {
    case "event_type":
      return search.event_type;
    case "status":
      return search.status;
    case "actor":
      return search.actor;
    // Date and text filters have no checkable options, so no CSV param: the
    // date lives in created_*, the text in custom_filters.
    default:
      return undefined;
  }
}

export function getAdminAuditLogFilterValues(
  search: AdminAuditLogSearchState,
  key: AdminAuditLogFilterKey,
): readonly string[] {
  if (key === "created_at") {
    const createdDates = canonicalCalendarDateCsv(search.created_dates);
    if (createdDates) return ["dates", ...createdDates.split(",")];
    return search.created_from || search.created_to
      ? ["range", search.created_from ?? "", search.created_to ?? ""]
      : [];
  }
  return getAdminAuditLogFilterCsv(search, key)?.split(",") ?? [];
}

export type AppliedAdminAuditLogFilter =
  AppliedDataTableFilter<AdminAuditLogFilterKey>;

const DATE_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});

function formatCalendarDate(value: string): string {
  const date = calendarDateObject(value);
  return date ? DATE_FORMATTER.format(date) : value;
}

function appliedDateFilter(
  field: AdminAuditLogFilterField,
  values: readonly string[],
): AppliedAdminAuditLogFilter | undefined {
  const [mode, ...dateValues] = values;
  if (mode === "dates") {
    if (dateValues.length === 0) return undefined;
    const valueLabels = dateValues.map(formatCalendarDate);
    const hiddenValueCount = Math.max(0, valueLabels.length - 2);
    return {
      field,
      values,
      valueLabels,
      operatorLabel: valueLabels.length === 1 ? "is on" : "is on any of",
      valueSummary: `${valueLabels.slice(0, 2).join(", ")}${
        hiddenValueCount > 0 ? ` +${String(hiddenValueCount)} more` : ""
      }`,
    };
  }
  if (mode !== "range") return undefined;

  const [from = "", to = ""] = dateValues;
  if (!from && !to) return undefined;

  if (from && to && from === to) {
    const date = formatCalendarDate(from);
    return {
      field,
      values,
      valueLabels: [date],
      operatorLabel: "is on",
      valueSummary: date,
    };
  }
  if (from && to) {
    const fromLabel = formatCalendarDate(from);
    const toLabel = formatCalendarDate(to);
    return {
      field,
      values,
      valueLabels: [fromLabel, toLabel],
      operatorLabel: "is between",
      valueSummary: `${fromLabel} and ${toLabel}`,
    };
  }
  if (from) {
    const date = formatCalendarDate(from);
    return {
      field,
      values,
      valueLabels: [date],
      operatorLabel: "is on or after",
      valueSummary: date,
    };
  }

  const date = formatCalendarDate(to);
  return {
    field,
    values,
    valueLabels: [date],
    operatorLabel: "is on or before",
    valueSummary: date,
  };
}

export function getAppliedAdminAuditLogFilters(
  fields: readonly AdminAuditLogFilterField[],
  search: AdminAuditLogSearchState,
): readonly AppliedAdminAuditLogFilter[] {
  return fields.flatMap((field) => {
    const applied: AppliedAdminAuditLogFilter[] = [];
    const values = getAdminAuditLogFilterValues(search, field.key);

    if (field.key === "created_at") {
      const dateFilter = values.length > 0 && appliedDateFilter(field, values);
      return dateFilter ? [dateFilter] : [];
    }

    if (values.length > 0) {
      applied.push({
        field,
        values,
        valueLabels: values.map(
          (value) =>
            field.options.find((item) => item.value === value)?.label ?? value,
        ),
      });
    }

    // Custom text is its own chip: it reads as `contains`, and clearing it must
    // not also clear the options the user checked on the same filter.
    const customValues = getAdminAuditLogCustomValues(search, field.key);
    if (customValues.length > 0) {
      applied.push({
        field,
        values: customValues,
        valueLabels: customValues,
        operatorLabel:
          customValues.length === 1 ? "contains" : "contains any of",
        custom: true,
      });
    }

    return applied;
  });
}

export function adminAuditLogFilterPatch(
  key: AdminAuditLogFilterKey,
  values: readonly string[] | undefined,
): Partial<AdminAuditLogSearchState> | undefined {
  // A text filter's state lives entirely in `custom_filters`, so it never
  // contributes a param of its own -- clearing or setting it is a no-op here.
  if (isTextFilterKey(key)) return {};

  if (values === undefined || values.length === 0) {
    if (key === "created_at") {
      return {
        created_dates: undefined,
        created_from: undefined,
        created_to: undefined,
      };
    }
    return { [key]: undefined };
  }

  const raw = values.join(",");
  switch (key) {
    case "event_type": {
      const selection = canonicalOpenCsv(raw);
      return selection === undefined ? undefined : { event_type: selection };
    }
    case "status": {
      const selection = canonicalKnownCsv(raw, STATUS_FILTERS);
      return selection === undefined ? undefined : { status: selection };
    }
    case "actor": {
      const selection = canonicalKnownCsv(raw, ACTOR_FILTERS);
      return selection === undefined ? undefined : { actor: selection };
    }
    case "created_at": {
      const [mode, ...dateValues] = values;
      if (mode === "dates") {
        if (dateValues.length === 0) {
          return {
            created_dates: undefined,
            created_from: undefined,
            created_to: undefined,
          };
        }
        const createdDates = canonicalCalendarDateCsv(dateValues);
        if (!createdDates) return undefined;
        return {
          created_dates: createdDates,
          created_from: undefined,
          created_to: undefined,
        };
      }
      if (mode !== "range") return undefined;

      const [rawFrom = "", rawTo = "", ...extra] = dateValues;
      if (extra.length > 0) return undefined;
      if (rawFrom === "" && rawTo === "") {
        return {
          created_dates: undefined,
          created_from: undefined,
          created_to: undefined,
        };
      }
      const createdFrom = rawFrom === "" ? undefined : calendarDate(rawFrom);
      const createdTo = rawTo === "" ? undefined : calendarDate(rawTo);
      if (
        (rawFrom !== "" && createdFrom === undefined) ||
        (rawTo !== "" && createdTo === undefined) ||
        (!createdFrom && !createdTo) ||
        (createdFrom && createdTo && createdFrom > createdTo)
      ) {
        return undefined;
      }
      return {
        created_dates: undefined,
        created_from: createdFrom,
        created_to: createdTo,
      };
    }
  }
}

function calendarDateObject(value: unknown): Date | undefined {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    return undefined;
  }
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const day = Number(value.slice(8, 10));
  const parsed = new Date(0);
  parsed.setUTCHours(0, 0, 0, 0);
  parsed.setUTCFullYear(year, month - 1, day);
  return parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day
    ? parsed
    : undefined;
}

function calendarDate(value: unknown): string | undefined {
  return typeof value === "string" && calendarDateObject(value)
    ? value
    : undefined;
}

export function isAdminAuditLogDateFilterValid(
  values: readonly string[],
): boolean {
  return adminAuditLogFilterPatch("created_at", values) !== undefined;
}

function positiveInteger(value: unknown): number | undefined {
  const parsed =
    typeof value === "number"
      ? value
      : typeof value === "string" && value.trim() !== ""
        ? Number(value)
        : Number.NaN;
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined;
}

function oneOf<T extends string>(
  value: unknown,
  options: readonly T[],
): T | undefined {
  return typeof value === "string" && options.includes(value as T)
    ? (value as T)
    : undefined;
}

function csvParts(value: unknown): readonly string[] | undefined {
  const rawValues = Array.isArray(value) ? value : [value];
  const parts: string[] = [];

  for (const rawValue of rawValues) {
    if (typeof rawValue !== "string") return undefined;

    for (const rawPart of rawValue.split(",")) {
      const part = rawPart.trim();
      if (part === "") return undefined;
      parts.push(part);
    }
  }

  return parts.length > 0 && parts.length <= MAX_MULTI_FILTER_VALUES
    ? parts
    : undefined;
}

function canonicalKnownCsv<T extends string>(
  value: unknown,
  options: readonly T[],
): string | undefined {
  const parts = csvParts(value);
  if (!parts || parts.some((part) => !options.includes(part as T))) {
    return undefined;
  }

  const selected = new Set(parts);
  return options.filter((option) => selected.has(option)).join(",");
}

/**
 * Canonicalizes a filter whose domain is open (event types are discovered from
 * the data, so there is no fixed list to check against). Values are bounded and
 * deduplicated, then sorted so the same selection always encodes to the same URL.
 * The server matches them exactly, never as a pattern.
 */
function canonicalOpenCsv(value: unknown): string | undefined {
  const parts = csvParts(value);
  if (
    !parts ||
    parts.some((part) => [...part].length > MAX_SEARCH_VALUE_LENGTH)
  ) {
    return undefined;
  }
  return [...new Set(parts)].sort((left, right) => left.localeCompare(right)).join(",");
}

function canonicalCalendarDateCsv(value: unknown): string | undefined {
  const parts = csvParts(value);
  if (!parts) return undefined;
  const dates = parts.map(calendarDate);
  if (dates.some((date) => date === undefined)) return undefined;
  return [...new Set(dates as string[])].sort().join(",");
}

export function normalizeAdminAuditLogSearch(
  raw: Record<string, unknown>,
): AdminAuditLogSearchState {
  const page = positiveInteger(raw.page);
  const perPage = positiveInteger(raw.per_page);
  const search = typeof raw.search === "string" ? raw.search.trim() : "";
  const searchFilters = parseAdminAuditLogSearchFilters(raw.search_filters);
  const customFilters = parseAdminAuditLogCustomFilters(raw.custom_filters);
  const eventType = canonicalOpenCsv(raw.event_type);
  const status = canonicalKnownCsv(raw.status, STATUS_FILTERS);
  const actor = canonicalKnownCsv(raw.actor, ACTOR_FILTERS);
  const createdDates = canonicalCalendarDateCsv(raw.created_dates);
  const createdFrom = calendarDate(raw.created_from);
  const createdTo = calendarDate(raw.created_to);
  const hasValidDateRange =
    createdFrom === undefined ||
    createdTo === undefined ||
    createdFrom <= createdTo;
  const sort = oneOf<AdminAuditLogSort>(raw.sort, SORTS);

  return {
    ...(page !== undefined && page > 1 ? { page } : {}),
    ...(perPage === 50 || perPage === 100 ? { per_page: perPage } : {}),
    ...(search !== "" && [...search].length <= 256 ? { search } : {}),
    ...(searchFilters !== undefined
      ? { search_filters: JSON.stringify(searchFilters) }
      : {}),
    ...(customFilters !== undefined
      ? { custom_filters: JSON.stringify(customFilters) }
      : {}),
    ...(eventType !== undefined ? { event_type: eventType } : {}),
    ...(status !== undefined ? { status } : {}),
    ...(actor !== undefined ? { actor } : {}),
    ...(createdDates !== undefined ? { created_dates: createdDates } : {}),
    ...(createdDates === undefined &&
    hasValidDateRange &&
    createdFrom !== undefined
      ? { created_from: createdFrom }
      : {}),
    ...(createdDates === undefined &&
    hasValidDateRange &&
    createdTo !== undefined
      ? { created_to: createdTo }
      : {}),
    ...(sort !== undefined && sort !== "-created_at" ? { sort } : {}),
  };
}
