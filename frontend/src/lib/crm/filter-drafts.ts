import { isCrmFilterOperatorAllowed } from "@/lib/crm/field-kinds";
import type { CrmFilter, CrmObjectMetadata } from "@/schemas/crm";

function hasOperand(filter: CrmFilter): boolean {
  if (filter.op === "is_set" || filter.op === "is_unset") return true;
  if (Array.isArray(filter.value)) return filter.value.length > 0;
  if (typeof filter.value === "string") return filter.value.trim().length > 0;
  return filter.value !== null && filter.value !== undefined;
}

export function executableCrmFilters(
  object: CrmObjectMetadata,
  drafts: readonly CrmFilter[],
): CrmFilter[] {
  return drafts.filter((filter) => {
    const field = object.fields.find(
      (candidate) => candidate.key === filter.field && !candidate.archived,
    );
    return Boolean(
      field && isCrmFilterOperatorAllowed(field, filter.op) && hasOperand(filter),
    );
  });
}
