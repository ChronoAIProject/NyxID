import type { CreateEndpointFormData } from "@/schemas/endpoints";
export interface CreateEndpointPayload {
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
