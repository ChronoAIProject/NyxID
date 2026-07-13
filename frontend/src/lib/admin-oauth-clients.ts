import type {
  AdminOAuthClientBrokerFilter,
  AdminOAuthClientCreatorType,
  AdminOAuthClientFilterField,
  AdminOAuthClientFilterKey,
  AdminOAuthClientFilterOptions,
  AdminOAuthClientPerPage,
  AdminOAuthClientSearchField,
  AdminOAuthClientSearchFieldKey,
  AdminOAuthClientSearchFilter,
  AdminOAuthClientSearchState,
  AdminOAuthClientSort,
  AdminOAuthClientTypeFilter,
} from "@/types/admin";
import type { AppliedDataTableFilter } from "@/types/data-table";

export const ADMIN_OAUTH_CLIENT_SCOPES = [
  "openid",
  "profile",
  "email",
  "roles",
  "groups",
  "offline_access",
  "proxy",
  "urn:nyxid:scope:broker_binding",
] as const;

export const ADMIN_OAUTH_CLIENT_PER_PAGE_OPTIONS = [
  10, 25, 50, 100,
] as const satisfies readonly AdminOAuthClientPerPage[];

export const ADMIN_OAUTH_CLIENT_DEFAULT_PER_PAGE: AdminOAuthClientPerPage = 25;

const CLIENT_TYPES = ["public", "confidential", "other"] as const;
const CREATOR_TYPES = [
  "dynamic_registration",
  "system",
  "owned",
  "ownerless",
] as const;
const BROKER_FILTERS = ["enabled", "disabled", "flag", "scope"] as const;
const STATUS_FILTERS = ["true", "false"] as const;
const MAX_MULTI_FILTER_VALUES = 32;
const MAX_SEARCH_GROUPS = 5;
const MAX_SEARCH_VALUES_PER_GROUP = 8;
const MAX_SEARCH_VALUES = 32;
const MAX_SEARCH_VALUE_LENGTH = 256;
const SORTS = [
  "-created_at",
  "created_at",
  "client_name",
  "-client_name",
  "client_type",
  "-client_type",
  "created_by",
  "-created_by",
  "broker",
  "-broker",
  "allowed_scopes",
  "-allowed_scopes",
  "-is_active",
  "is_active",
] as const;

const FILTER_KEYS = [
  "is_active",
  "client_type",
  "creator_type",
  "broker",
  "scope",
  "created_at",
] as const;

export const ADMIN_OAUTH_CLIENT_SEARCH_FIELDS = [
  { key: "client", label: "Client" },
  { key: "client_type", label: "Client type" },
  { key: "created_by", label: "Created by" },
  { key: "allowed_scopes", label: "Allowed scopes" },
] as const satisfies readonly AdminOAuthClientSearchField[];

const SEARCH_FIELD_KEYS = ADMIN_OAUTH_CLIENT_SEARCH_FIELDS.map(
  ({ key }) => key,
);

const CLIENT_TYPE_LABELS: Record<AdminOAuthClientTypeFilter, string> = {
  public: "Public",
  confidential: "Confidential",
  other: "Other",
};

const CREATOR_TYPE_LABELS: Record<AdminOAuthClientCreatorType, string> = {
  dynamic_registration: "Dynamic registration",
  system: "System",
  owned: "User / org",
  ownerless: "Ownerless",
};

const BROKER_FILTER_LABELS: Record<AdminOAuthClientBrokerFilter, string> = {
  enabled: "Enabled",
  disabled: "Disabled",
  flag: "Enabled by admin grant",
  scope: "Enabled by broker scope",
};

const SCOPE_LABELS: Readonly<Record<string, string>> = {
  openid: "OpenID",
  profile: "Profile",
  email: "Email",
  roles: "Roles",
  groups: "Groups",
  offline_access: "Offline access",
  proxy: "Proxy",
  "urn:nyxid:scope:broker_binding": "Broker binding",
};

function option(value: string, label: string) {
  return { value, label } as const;
}

function fallbackFilterFields(
  filterOptions?: AdminOAuthClientFilterOptions,
): readonly AdminOAuthClientFilterField[] {
  const clientTypes = filterOptions?.client_types ?? CLIENT_TYPES;
  const creatorTypes = filterOptions?.creator_types ?? CREATOR_TYPES;
  const brokerFilters = filterOptions?.broker_filters ?? BROKER_FILTERS;
  const statuses = filterOptions?.statuses ?? [true, false];
  const allowedScopes =
    filterOptions?.allowed_scopes ?? ADMIN_OAUTH_CLIENT_SCOPES;

  return [
    {
      key: "is_active",
      label: "Status",
      value_type: "boolean",
      operator: "is",
      multiple: false,
      options: statuses.map((value) =>
        option(String(value), value ? "Active" : "Inactive"),
      ),
    },
    {
      key: "client_type",
      label: "Client type",
      value_type: "enum",
      operator: "is",
      multiple: false,
      options: clientTypes.map((value) =>
        option(value, CLIENT_TYPE_LABELS[value]),
      ),
    },
    {
      key: "creator_type",
      label: "Creator",
      value_type: "enum",
      operator: "is",
      multiple: false,
      options: creatorTypes.map((value) =>
        option(value, CREATOR_TYPE_LABELS[value]),
      ),
    },
    {
      key: "broker",
      label: "Broker capability",
      value_type: "enum",
      operator: "is",
      multiple: false,
      options: brokerFilters.map((value) =>
        option(value, BROKER_FILTER_LABELS[value]),
      ),
    },
    {
      key: "scope",
      label: "Allowed scope",
      value_type: "enum",
      operator: "includes",
      multiple: false,
      options: allowedScopes.map((value) =>
        option(value, SCOPE_LABELS[value] ?? value),
      ),
    },
  ];
}

function isFilterKey(value: unknown): value is AdminOAuthClientFilterKey {
  return (
    typeof value === "string" &&
    FILTER_KEYS.includes(value as AdminOAuthClientFilterKey)
  );
}

/**
 * Uses the server-described fields when available and fills any missing field
 * from the legacy option arrays during rolling deployments.
 */
export function getAdminOAuthClientFilterFields(
  filterOptions?: AdminOAuthClientFilterOptions,
): readonly AdminOAuthClientFilterField[] {
  const fallbacks = fallbackFilterFields(filterOptions);
  const serverFields = filterOptions?.fields ?? [];
  const seen = new Set<AdminOAuthClientFilterKey>();
  const fields: AdminOAuthClientFilterField[] = [];

  for (const field of serverFields) {
    const isDateField =
      field.key === "created_at" &&
      field.value_type === "date" &&
      field.operator === "between" &&
      Array.isArray(field.options) &&
      field.options.length === 0;
    const isOptionField =
      field.key !== "created_at" &&
      (field.value_type === "enum" || field.value_type === "boolean") &&
      (field.operator === "is" || field.operator === "includes") &&
      Array.isArray(field.options) &&
      field.options.length > 0;

    if (
      !isFilterKey(field.key) ||
      seen.has(field.key) ||
      (!isDateField && !isOptionField)
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
    if (!isDateField && options.length === 0) continue;

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

  return fields;
}

function isSearchFieldKey(
  value: unknown,
): value is AdminOAuthClientSearchFieldKey {
  return (
    typeof value === "string" &&
    SEARCH_FIELD_KEYS.includes(value as AdminOAuthClientSearchFieldKey)
  );
}

/**
 * Returns server-advertised search fields in stable table order. Scoped
 * search stays hidden for older backends that do not advertise support.
 */
export function getAdminOAuthClientSearchFields(
  filterOptions?: AdminOAuthClientFilterOptions,
): readonly AdminOAuthClientSearchField[] {
  if (filterOptions?.search_fields === undefined) {
    return [];
  }

  const fieldsByKey = new Map<
    AdminOAuthClientSearchFieldKey,
    AdminOAuthClientSearchField
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
): readonly AdminOAuthClientSearchFilter[] | undefined {
  if (
    !Array.isArray(value) ||
    value.length === 0 ||
    value.length > MAX_SEARCH_GROUPS
  ) {
    return undefined;
  }

  const fields = new Set<AdminOAuthClientSearchFieldKey>();
  const groups: AdminOAuthClientSearchFilter[] = [];
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
export function parseAdminOAuthClientSearchFilters(
  raw: unknown,
): readonly AdminOAuthClientSearchFilter[] | undefined {
  if (Array.isArray(raw)) return normalizeSearchFilterGroups(raw);
  if (typeof raw !== "string" || raw === "") return undefined;
  try {
    return normalizeSearchFilterGroups(JSON.parse(raw));
  } catch {
    return undefined;
  }
}

/** Encodes validated field groups as the canonical API/URL JSON value. */
export function encodeAdminOAuthClientSearchFilters(
  filters: readonly AdminOAuthClientSearchFilter[],
): string | undefined {
  const normalized = normalizeSearchFilterGroups(filters);
  return normalized ? JSON.stringify(normalized) : undefined;
}

export function getAdminOAuthClientSearchFilters(
  search: AdminOAuthClientSearchState,
): readonly AdminOAuthClientSearchFilter[] {
  return parseAdminOAuthClientSearchFilters(search.search_filters) ?? [];
}

/** Upserts or removes one field group and returns canonical URL state. */
export function updateAdminOAuthClientSearchFilters(
  current: unknown,
  field: AdminOAuthClientSearchFieldKey,
  values: readonly string[] | undefined,
): string | undefined {
  const filters = parseAdminOAuthClientSearchFilters(current) ?? [];
  const remaining = filters.filter((filter) => filter.field !== field);
  if (values === undefined || values.length === 0) {
    return remaining.length > 0
      ? encodeAdminOAuthClientSearchFilters(remaining)
      : undefined;
  }
  return encodeAdminOAuthClientSearchFilters([...remaining, { field, values }]);
}

/**
 * Filters that accept free text, and stay in lockstep with the backend's
 * `ADMIN_CUSTOM_TEXT_FILTERS`. A filter can only take custom text when it maps
 * to a single string column the server can run a `contains` against, which
 * rules out `is_active` (boolean), `broker` (derived), and `created_at` (date).
 */
export const ADMIN_OAUTH_CLIENT_CUSTOM_TEXT_FILTERS = [
  "client_type",
  "creator_type",
  "scope",
] as const satisfies readonly AdminOAuthClientFilterKey[];

export type AdminOAuthClientCustomFilters = Partial<
  Record<AdminOAuthClientFilterKey, readonly string[]>
>;

function isCustomTextFilterKey(value: unknown): value is AdminOAuthClientFilterKey {
  return (ADMIN_OAUTH_CLIENT_CUSTOM_TEXT_FILTERS as readonly string[]).includes(
    value as string,
  );
}

function normalizeCustomFilters(
  value: unknown,
): AdminOAuthClientCustomFilters | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined;
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0 || entries.length > MAX_SEARCH_GROUPS) return undefined;

  const normalized: Record<string, readonly string[]> = {};
  let totalValues = 0;

  // Emit keys in a fixed order so the same selection always encodes to the same
  // URL, whatever order the user filled the filters in.
  for (const key of ADMIN_OAUTH_CLIENT_CUSTOM_TEXT_FILTERS) {
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
export function parseAdminOAuthClientCustomFilters(
  raw: unknown,
): AdminOAuthClientCustomFilters | undefined {
  if (typeof raw === "object" && raw !== null) return normalizeCustomFilters(raw);
  if (typeof raw !== "string" || raw === "") return undefined;
  try {
    return normalizeCustomFilters(JSON.parse(raw));
  } catch {
    return undefined;
  }
}

/** Encodes validated custom text as the canonical API/URL JSON value. */
export function encodeAdminOAuthClientCustomFilters(
  filters: AdminOAuthClientCustomFilters,
): string | undefined {
  const normalized = normalizeCustomFilters(filters);
  return normalized ? JSON.stringify(normalized) : undefined;
}

export function getAdminOAuthClientCustomValues(
  search: AdminOAuthClientSearchState,
  key: AdminOAuthClientFilterKey,
): readonly string[] {
  return parseAdminOAuthClientCustomFilters(search.custom_filters)?.[key] ?? [];
}

/**
 * Replaces every filter's custom text and returns canonical URL state.
 *
 * The UI hands back a draft keyed by every filter it rendered, so prune the ones
 * that carry no text (and any filter that takes none) before encoding: the URL
 * form is strict and would reject the padded shape wholesale.
 */
export function adminOAuthClientCustomFilterPatch(
  filters: AdminOAuthClientCustomFilters,
): Partial<AdminOAuthClientSearchState> {
  const populated: AdminOAuthClientCustomFilters = {};
  for (const key of ADMIN_OAUTH_CLIENT_CUSTOM_TEXT_FILTERS) {
    const values = filters[key];
    if (values && values.length > 0) populated[key] = values;
  }
  return { custom_filters: encodeAdminOAuthClientCustomFilters(populated) };
}

function getAdminOAuthClientFilterCsv(
  search: AdminOAuthClientSearchState,
  key: AdminOAuthClientFilterKey,
): string | undefined {
  switch (key) {
    case "is_active":
      return search.is_active === undefined
        ? undefined
        : String(search.is_active);
    case "client_type":
      return search.client_type;
    case "creator_type":
      return search.creator_type;
    case "broker":
      return search.broker;
    case "scope":
      return search.scope;
    case "created_at":
      return undefined;
  }
}

export function getAdminOAuthClientFilterValues(
  search: AdminOAuthClientSearchState,
  key: AdminOAuthClientFilterKey,
): readonly string[] {
  if (key === "created_at") {
    const createdDates = canonicalCalendarDateCsv(search.created_dates);
    if (createdDates) return ["dates", ...createdDates.split(",")];
    return search.created_from || search.created_to
      ? ["range", search.created_from ?? "", search.created_to ?? ""]
      : [];
  }
  return getAdminOAuthClientFilterCsv(search, key)?.split(",") ?? [];
}

export type AppliedAdminOAuthClientFilter =
  AppliedDataTableFilter<AdminOAuthClientFilterKey>;

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
  field: AdminOAuthClientFilterField,
  values: readonly string[],
): AppliedAdminOAuthClientFilter | undefined {
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

export function getAppliedAdminOAuthClientFilters(
  fields: readonly AdminOAuthClientFilterField[],
  search: AdminOAuthClientSearchState,
): readonly AppliedAdminOAuthClientFilter[] {
  return fields.flatMap((field) => {
    const applied: AppliedAdminOAuthClientFilter[] = [];
    const values = getAdminOAuthClientFilterValues(search, field.key);

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
    const customValues = getAdminOAuthClientCustomValues(search, field.key);
    if (customValues.length > 0) {
      applied.push({
        field,
        values: customValues,
        valueLabels: customValues,
        operatorLabel: customValues.length === 1 ? "contains" : "contains any of",
        custom: true,
      });
    }

    return applied;
  });
}

export function adminOAuthClientFilterPatch(
  key: AdminOAuthClientFilterKey,
  values: readonly string[] | undefined,
): Partial<AdminOAuthClientSearchState> | undefined {
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
    case "is_active": {
      const selection = canonicalKnownCsv(raw, STATUS_FILTERS);
      if (selection === undefined) return undefined;
      return {
        is_active: selection.includes(",") ? selection : selection === "true",
      };
    }
    case "client_type": {
      const selection = canonicalKnownCsv(raw, CLIENT_TYPES);
      return selection === undefined ? undefined : { client_type: selection };
    }
    case "creator_type": {
      const selection = canonicalKnownCsv(raw, CREATOR_TYPES);
      return selection === undefined ? undefined : { creator_type: selection };
    }
    case "broker": {
      const selection = canonicalKnownCsv(raw, BROKER_FILTERS);
      return selection === undefined ? undefined : { broker: selection };
    }
    case "scope": {
      const selection = canonicalScopeCsv(raw);
      return selection === undefined ? undefined : { scope: selection };
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

export function isAdminOAuthClientDateFilterValid(
  values: readonly string[],
): boolean {
  return adminOAuthClientFilterPatch("created_at", values) !== undefined;
}

function perPage(value: unknown): AdminOAuthClientPerPage | undefined {
  const parsed = positiveInteger(value);
  return ADMIN_OAUTH_CLIENT_PER_PAGE_OPTIONS.find(
    (option) => option === parsed,
  );
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
    if (typeof rawValue === "boolean") {
      parts.push(String(rawValue));
      continue;
    }
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

function scopeToken(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const scope = value.trim();
  return scope.length <= 256 && /^[\x21\x23-\x5b\x5d-\x7e]+$/.test(scope)
    ? scope
    : undefined;
}

function canonicalScopeCsv(value: unknown): string | undefined {
  const parts = csvParts(value);
  if (!parts) return undefined;

  const scopes = parts.map(scopeToken);
  if (scopes.some((scope) => scope === undefined)) return undefined;

  const selected = new Set(scopes as string[]);
  const known = ADMIN_OAUTH_CLIENT_SCOPES.filter((scope) =>
    selected.delete(scope),
  );
  const future = [...selected].sort((left, right) => left.localeCompare(right));
  return [...known, ...future].join(",");
}

function canonicalCalendarDateCsv(value: unknown): string | undefined {
  const parts = csvParts(value);
  if (!parts) return undefined;
  const dates = parts.map(calendarDate);
  if (dates.some((date) => date === undefined)) return undefined;
  return [...new Set(dates as string[])].sort().join(",");
}

export function normalizeAdminOAuthClientSearch(
  raw: Record<string, unknown>,
): AdminOAuthClientSearchState {
  const page = positiveInteger(raw.page);
  const rowsPerPage = perPage(raw.per_page);
  const search = typeof raw.search === "string" ? raw.search.trim() : "";
  const searchFilters = parseAdminOAuthClientSearchFilters(raw.search_filters);
  const customFilters = parseAdminOAuthClientCustomFilters(raw.custom_filters);
  const clientType = canonicalKnownCsv(raw.client_type, CLIENT_TYPES);
  const creatorType = canonicalKnownCsv(raw.creator_type, CREATOR_TYPES);
  const broker = canonicalKnownCsv(raw.broker, BROKER_FILTERS);
  const isActiveCsv = canonicalKnownCsv(raw.is_active, STATUS_FILTERS);
  const isActive =
    isActiveCsv === undefined
      ? undefined
      : isActiveCsv.includes(",")
        ? isActiveCsv
        : isActiveCsv === "true";
  const scope = canonicalScopeCsv(raw.scope);
  const createdDates = canonicalCalendarDateCsv(raw.created_dates);
  const createdFrom = calendarDate(raw.created_from);
  const createdTo = calendarDate(raw.created_to);
  const hasValidDateRange =
    createdFrom === undefined ||
    createdTo === undefined ||
    createdFrom <= createdTo;
  const sort = oneOf<AdminOAuthClientSort>(raw.sort, SORTS);

  return {
    ...(page !== undefined && page > 1 ? { page } : {}),
    ...(rowsPerPage !== undefined &&
    rowsPerPage !== ADMIN_OAUTH_CLIENT_DEFAULT_PER_PAGE
      ? { per_page: rowsPerPage }
      : {}),
    ...(search !== "" && [...search].length <= 256 ? { search } : {}),
    ...(searchFilters !== undefined
      ? { search_filters: JSON.stringify(searchFilters) }
      : {}),
    ...(customFilters !== undefined
      ? { custom_filters: JSON.stringify(customFilters) }
      : {}),
    ...(clientType !== undefined ? { client_type: clientType } : {}),
    ...(creatorType !== undefined ? { creator_type: creatorType } : {}),
    ...(broker !== undefined ? { broker } : {}),
    ...(isActive !== undefined ? { is_active: isActive } : {}),
    ...(scope !== undefined ? { scope } : {}),
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
