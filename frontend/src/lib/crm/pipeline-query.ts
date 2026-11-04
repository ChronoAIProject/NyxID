import type {
  CrmAggregateQuery,
  CrmFilter,
  CrmTableQuery,
} from "@/schemas/crm";
import { crmTableQuerySchema } from "@/schemas/crm";

export type CrmPipelineLanePlan =
  | { readonly kind: "query"; readonly query: CrmTableQuery }
  | { readonly kind: "empty" }
  | { readonly kind: "invalid"; readonly error: Error };

function stageFilter(
  stageField: string,
  stageValues: readonly string[],
): CrmFilter {
  return stageValues.length > 1
    ? { field: stageField, op: "in", value: [...stageValues] }
    : { field: stageField, op: "eq", value: stageValues[0] };
}

export function crmPipelineLaneQuery(
  query: CrmTableQuery,
  stageField: string,
  stageValues: readonly string[],
): CrmPipelineLanePlan {
  const base = crmTableQuerySchema.safeParse(query);
  if (!base.success) return { kind: "invalid", error: base.error };

  const allowedStages = [...new Set(stageValues)].filter((stageValue) =>
    crmPipelineStageMatches(query.filters, stageField, stageValue),
  );
  if (allowedStages.length === 0) return { kind: "empty" };

  const foldedStage = stageFilter(stageField, allowedStages);
  let stageInserted = false;
  const filters = query.filters.flatMap((filter) => {
    if (filter.field !== stageField) return [filter];
    if (stageInserted) return [];
    stageInserted = true;
    return [foldedStage];
  });
  if (!stageInserted) filters.push(foldedStage);

  const laneQuery: CrmTableQuery = {
    ...query,
    cursor: undefined,
    page_size: Math.min(query.page_size ?? 50, 50),
    filters,
  };
  const parsed = crmTableQuerySchema.safeParse(laneQuery);
  return parsed.success
    ? { kind: "query", query: laneQuery }
    : { kind: "invalid", error: parsed.error };
}

export function crmPipelineCountQuery(
  query: CrmTableQuery,
  stageField: string,
): CrmAggregateQuery {
  return {
    object: query.object,
    filters: [...query.filters],
    group_by: stageField,
    measure: { op: "count" },
    order: "option_order",
  };
}

export function crmPipelineStageMatches(
  filters: readonly CrmFilter[],
  stageField: string,
  stageValue: string,
): boolean {
  return filters
    .filter((filter) => filter.field === stageField)
    .every((filter) => {
      if (filter.op === "eq") return filter.value === stageValue;
      if (filter.op === "neq") return filter.value !== stageValue;
      if (filter.op === "in")
        return Array.isArray(filter.value) && filter.value.includes(stageValue);
      if (filter.op === "nin")
        return Array.isArray(filter.value) && !filter.value.includes(stageValue);
      if (filter.op === "is_set") return true;
      if (filter.op === "is_unset") return false;
      return false;
    });
}
