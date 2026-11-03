import type {
  CrmAggregateQuery,
  CrmFilter,
  CrmTableQuery,
} from "@/schemas/crm";

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
): CrmTableQuery {
  return {
    ...query,
    cursor: undefined,
    page_size: Math.min(query.page_size ?? 50, 50),
    filters: [...query.filters, stageFilter(stageField, stageValues)],
  };
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
