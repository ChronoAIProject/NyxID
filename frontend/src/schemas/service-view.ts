import { z } from "zod";

const selectionId = z
  .string()
  .refine(
    (value) => value.trim().length > 0 && Array.from(value).length <= 128,
    "Selections must contain 1 to 128 characters",
  );
const selections = z
  .array(selectionId)
  .max(100)
  .transform((ids) => [...new Set(ids)]);

export const serviceViewSchema = z
  .object({
    search: z
      .string()
      .refine(
        (value) => Array.from(value).length <= 200,
        "Search is limited to 200 characters",
      ),
    organization_ids: selections.optional(),
    service_group_ids: selections.optional(),
    organization_id: selectionId.nullish(),
    service_group_id: selectionId.nullish(),
    source: z.enum(["all", "personal", "org", "platform"]),
    state: z.enum(["all", "enabled", "disabled"]),
    service_type: z.enum(["all", "http", "ssh"]),
    show_auto_connected: z.boolean(),
  })
  .strict()
  .refine(
    (value) =>
      !(
        value.organization_ids !== undefined &&
        value.organization_id !== undefined
      ),
    "Use organization_ids",
  )
  .refine(
    (value) =>
      !(
        value.service_group_ids !== undefined &&
        value.service_group_id !== undefined
      ),
    "Use service_group_ids",
  )
  .transform(
    ({
      organization_id,
      service_group_id,
      organization_ids,
      service_group_ids,
      ...filters
    }) => ({
      ...filters,
      organization_ids:
        organization_ids ?? (organization_id ? [organization_id] : []),
      service_group_ids:
        service_group_ids ?? (service_group_id ? [service_group_id] : []),
    }),
  );

export type ServiceViewFilters = z.infer<typeof serviceViewSchema>;

export const DEFAULT_SERVICE_FILTERS: ServiceViewFilters = {
  search: "",
  organization_ids: [],
  service_group_ids: [],
  source: "personal",
  state: "all",
  service_type: "all",
  show_auto_connected: true,
};

export function sameServiceFilters(
  a: ServiceViewFilters,
  b: ServiceViewFilters,
): boolean {
  return (
    a.search === b.search &&
    a.organization_ids.length === b.organization_ids.length &&
    a.organization_ids.every((id) => b.organization_ids.includes(id)) &&
    a.service_group_ids.length === b.service_group_ids.length &&
    a.service_group_ids.every((id) => b.service_group_ids.includes(id)) &&
    a.source === b.source &&
    a.state === b.state &&
    a.service_type === b.service_type &&
    a.show_auto_connected === b.show_auto_connected
  );
}
