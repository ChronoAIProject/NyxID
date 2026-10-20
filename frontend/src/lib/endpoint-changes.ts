import type { CreateEndpointFormData } from "@/schemas/endpoints";
export interface CreateEndpointPayload {
  readonly data_scope?: CreateEndpointFormData["data_scope"];
  readonly cost_class?: CreateEndpointFormData["cost_class"];
  readonly execution?: CreateEndpointFormData["execution"];
  readonly name: string;
  readonly description?: string | null;
  readonly method: string;
  readonly path: string;
  readonly parameters?: unknown | null;
  readonly request_body_schema?: unknown | null;
  readonly response_description?: string | null;
}

export function formToPayload(
  data: CreateEndpointFormData,
): CreateEndpointPayload {
  return {
    data_scope: data.data_scope,
    cost_class: data.cost_class,
    execution: data.execution,
    name: data.name,
    description: data.description || null,
    method: data.method,
    path: data.path,
    parameters: data.parameters ? JSON.parse(data.parameters) : null,
    request_body_schema: data.request_body_schema
      ? JSON.parse(data.request_body_schema)
      : null,
    response_description: data.response_description || null,
  };
}
