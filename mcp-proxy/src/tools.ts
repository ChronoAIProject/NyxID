import type {
  McpConfig,
  McpEndpoint,
  McpService,
  EndpointParameter,
} from './types.js';

export interface ToolDefinition {
  readonly name: string;
  readonly description: string;
  readonly inputSchema: {
    readonly type: 'object';
    readonly properties: Record<string, unknown>;
    readonly required?: readonly string[];
  };
}

export interface ResolvedTool {
  readonly service: McpService;
  readonly endpoint: McpEndpoint;
}

export function generateToolDefinitions(
  config: McpConfig,
): readonly ToolDefinition[] {
  const tools: ToolDefinition[] = [];

  for (const service of config.services) {
    for (const endpoint of service.endpoints) {
      const toolName = `${service.slug}__${endpoint.name}`;
      const description = `[${service.name}] ${endpoint.description ?? endpoint.name}`;
      const inputSchema = buildInputSchema(endpoint);

      tools.push({ name: toolName, description, inputSchema });
    }
  }

  return tools;
}

export function resolveToolCall(
  name: string,
  config: McpConfig,
): ResolvedTool | null {
  const separatorIndex = name.indexOf('__');
  if (separatorIndex === -1) {
    return null;
  }

  const serviceSlug = name.slice(0, separatorIndex);
  const endpointName = name.slice(separatorIndex + 2);

  const service = config.services.find((s) => s.slug === serviceSlug);
  if (!service) {
    return null;
  }

  const endpoint = service.endpoints.find((e) => e.name === endpointName);
  if (!endpoint) {
    return null;
  }

  return { service, endpoint };
}

function buildInputSchema(
  endpoint: McpEndpoint,
): ToolDefinition['inputSchema'] {
  const properties: Record<string, unknown> = {};
  const required: string[] = [];

  if (endpoint.parameters) {
    for (const param of endpoint.parameters) {
      properties[param.name] = buildParameterSchema(param);
      if (param.required) {
        required.push(param.name);
      }
    }
  }

  if (endpoint.request_body_schema) {
    const bodySchema = endpoint.request_body_schema;

    if (
      bodySchema.type === 'object' &&
      typeof bodySchema.properties === 'object' &&
      bodySchema.properties !== null
    ) {
      const bodyProps = bodySchema.properties as Record<string, unknown>;
      for (const [key, value] of Object.entries(bodyProps)) {
        properties[key] = value;
      }
      if (Array.isArray(bodySchema.required)) {
        for (const req of bodySchema.required) {
          if (typeof req === 'string') {
            required.push(req);
          }
        }
      }
    } else {
      properties['body'] = {
        ...bodySchema,
        description: 'Request body',
      };
      required.push('body');
    }
  }

  return {
    type: 'object',
    properties,
    ...(required.length > 0 ? { required } : {}),
  };
}

function buildParameterSchema(
  param: EndpointParameter,
): Record<string, unknown> {
  const schema: Record<string, unknown> = {
    type: param.schema.type === 'integer' ? 'integer' : param.schema.type,
  };

  const desc = param.description ?? param.schema.description;
  if (desc) {
    schema.description = desc;
  }

  if (param.schema.format) {
    schema.format = param.schema.format;
  }

  if (param.schema.enum) {
    schema.enum = param.schema.enum;
  }

  if (param.schema.default !== undefined) {
    schema.default = param.schema.default;
  }

  return schema;
}

export function buildProxyArgs(
  endpoint: McpEndpoint,
  args: Record<string, unknown>,
): {
  readonly method: string;
  readonly path: string;
  readonly query: Record<string, string>;
  readonly body: unknown | undefined;
} {
  let path = endpoint.path.replace(/^\//, '');
  const query: Record<string, string> = {};
  const bodyFields: Record<string, unknown> = {};

  const pathParams = new Set<string>();
  const queryParams = new Set<string>();

  if (endpoint.parameters) {
    for (const param of endpoint.parameters) {
      if (param.in === 'path') {
        pathParams.add(param.name);
      } else if (param.in === 'query') {
        queryParams.add(param.name);
      }
    }
  }

  for (const [key, value] of Object.entries(args)) {
    if (pathParams.has(key)) {
      path = path.replace(`{${key}}`, String(value));
    } else if (queryParams.has(key)) {
      query[key] = String(value);
    } else {
      bodyFields[key] = value;
    }
  }

  const hasBody = Object.keys(bodyFields).length > 0;
  const body = hasBody
    ? bodyFields.body !== undefined
      ? bodyFields.body
      : bodyFields
    : undefined;

  return { method: endpoint.method, path, query, body };
}
