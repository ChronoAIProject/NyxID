import { z } from "zod";
import {
  crmViewComponentCatalog,
  isCrmViewComponentType,
} from "@/lib/crm/views/catalog";
import {
  crmViewEnvelopeBaseSchema,
  type CrmValidationError,
  type CrmViewEnvelope,
} from "@/lib/crm/views/schema";

export type CrmViewValidationResult =
  | { readonly success: true; readonly data: CrmViewEnvelope }
  | { readonly success: false; readonly errors: readonly CrmValidationError[] };

export interface CrmViewObjectMetadata {
  readonly key: string;
  readonly fields: readonly {
    readonly key: string;
    readonly archived: boolean;
  }[];
}

function pathToString(path: PropertyKey[]): string {
  return path.map(String).join(".");
}

function zodErrors(
  error: z.ZodError,
  prefix: PropertyKey[] = [],
): CrmValidationError[] {
  return error.issues.map((issue) => ({
    path: pathToString([...prefix, ...issue.path]),
    message: issue.message,
  }));
}

function validateGraph(
  envelope: CrmViewEnvelope,
  errors: CrmValidationError[],
) {
  const { root, elements } = envelope.spec;
  if (!elements[root]) {
    errors.push({
      path: "spec.root",
      message: "Root must reference an existing element",
    });
    return;
  }

  const parents = new Map<string, string>();
  let edgeCount = 0;
  let edgeLimitExceeded = false;
  for (const [elementId, element] of Object.entries(elements)) {
    edgeCount += element.children.length;
    if (edgeCount > 60 && !edgeLimitExceeded) {
      edgeLimitExceeded = true;
      errors.push({
        path: `spec.elements.${elementId}.children`,
        message: "A saved view can define at most 60 child edges",
      });
    }

    const children = new Set<string>();
    element.children.forEach((childId, index) => {
      const path = `spec.elements.${elementId}.children.${String(index)}`;
      if (children.has(childId)) {
        errors.push({
          path,
          message: `Child element '${childId}' must not be repeated`,
        });
        return;
      }
      children.add(childId);

      const parent = parents.get(childId);
      if (parent !== undefined && parent !== elementId) {
        errors.push({
          path,
          message: `Child element '${childId}' must not have multiple parents`,
        });
        return;
      }
      parents.set(childId, elementId);
    });
  }

  if (edgeLimitExceeded) return;

  const visiting = new Set<string>();
  const visited = new Set<string>();

  function visit(elementId: string, depth: number) {
    if (visiting.has(elementId)) {
      errors.push({
        path: `spec.elements.${elementId}.children`,
        message: "Element children must not contain a cycle",
      });
      return;
    }
    if (visited.has(elementId)) return;
    if (depth > 8) {
      errors.push({
        path: `spec.elements.${elementId}`,
        message: "Saved view depth must not exceed 8 elements",
      });
      return;
    }

    visiting.add(elementId);
    const element = elements[elementId];
    if (element) {
      element.children.forEach((childId, index) => {
        if (!elements[childId]) {
          errors.push({
            path: `spec.elements.${elementId}.children.${String(index)}`,
            message: `Unknown child element '${childId}'`,
          });
          return;
        }
        visit(childId, depth + 1);
      });
    }
    visiting.delete(elementId);
    visited.add(elementId);
  }

  visit(root, 1);
  for (const elementId of Object.keys(elements)) {
    if (!visited.has(elementId)) {
      errors.push({
        path: `spec.elements.${elementId}`,
        message: "Element is not reachable from spec.root",
      });
    }
  }
}

function validateQueryBinding(
  envelope: CrmViewEnvelope,
  elementId: string,
  componentType: keyof typeof crmViewComponentCatalog,
  props: Record<string, unknown>,
  errors: CrmValidationError[],
) {
  const requirement = crmViewComponentCatalog[componentType].query;
  if (requirement.kind === "none") return;

  const queryId = props.queryId;
  if (typeof queryId !== "string") return;
  const query = envelope.queries[queryId];
  const path = `spec.elements.${elementId}.props.queryId`;
  if (!query) {
    errors.push({ path, message: `Unknown query '${queryId}'` });
    return;
  }
  if (query.kind !== requirement.kind) {
    errors.push({
      path,
      message: `${componentType} requires a ${requirement.kind} query`,
    });
    return;
  }
  if (
    componentType === "PipelineBoard" &&
    query.kind === "table" &&
    query.query.include_archived
  )
    errors.push({
      path,
      message:
        "Pipeline boards exclude archived records; use ObjectTable for archived records",
    });
  if (
    componentType === "ObjectTable" &&
    query.kind === "table" &&
    query.query.select &&
    Array.isArray(props.columns) &&
    props.columns.some(
      (column) =>
        typeof column === "string" && !query.query.select?.includes(column),
    )
  )
    errors.push({
      path: `spec.elements.${elementId}.props.columns`,
      message: "Table columns must be included in query.select",
    });
  if (requirement.kind === "aggregate" && query.kind === "aggregate") {
    if (
      componentType === "PieChart" &&
      !["count", "sum"].includes(query.query.measure.op)
    ) {
      errors.push({
        path,
        message: "Pie charts require count or sum measures",
      });
    }
    const grouped = query.query.group_by !== undefined;
    if (grouped !== requirement.grouped) {
      errors.push({
        path,
        message: requirement.grouped
          ? `${componentType} requires a grouped aggregate query`
          : `${componentType} requires an aggregate query without group_by`,
      });
    }
  }
}

function validateLiveFields(
  envelope: CrmViewEnvelope,
  elementId: string,
  componentType: keyof typeof crmViewComponentCatalog,
  props: Record<string, unknown>,
  objects: readonly CrmViewObjectMetadata[] | undefined,
  errors: CrmValidationError[],
) {
  if (!objects) return;
  let objectKey: string | undefined;
  let fieldProperty: "columns" | "fields" | undefined;
  if (componentType === "ObjectTable") {
    const queryId = props.queryId;
    const query =
      typeof queryId === "string" ? envelope.queries[queryId] : undefined;
    if (query?.kind === "table") objectKey = query.query.object;
    fieldProperty = "columns";
  } else if (componentType === "RecordForm") {
    if (typeof props.object === "string") objectKey = props.object;
    fieldProperty = "fields";
  }
  if (!objectKey || !fieldProperty) return;
  const selectedFields = props[fieldProperty];
  if (!Array.isArray(selectedFields)) return;
  const object = objects.find((candidate) => candidate.key === objectKey);
  if (!object) return;
  const liveFields = new Set(
    object.fields.filter((field) => !field.archived).map((field) => field.key),
  );
  selectedFields.forEach((field, index) => {
    if (typeof field === "string" && !liveFields.has(field)) {
      errors.push({
        path: `spec.elements.${elementId}.props.${fieldProperty}.${String(index)}`,
        message: `Must reference a live field on '${objectKey}'`,
      });
    }
  });
}

export function validateCrmViewEnvelope(
  input: unknown,
  objects?: readonly CrmViewObjectMetadata[],
): CrmViewValidationResult {
  const parsed = crmViewEnvelopeBaseSchema.safeParse(input);
  if (!parsed.success) {
    return { success: false, errors: zodErrors(parsed.error) };
  }

  const errors: CrmValidationError[] = [];
  for (const [elementId, element] of Object.entries(
    parsed.data.spec.elements,
  )) {
    const typePath = ["spec", "elements", elementId, "type"];
    if (!isCrmViewComponentType(element.type)) {
      errors.push({
        path: pathToString(typePath),
        message: `Unknown component '${element.type}'`,
      });
      continue;
    }

    const definition = crmViewComponentCatalog[element.type];
    const props = definition.props.safeParse(element.props);
    if (!props.success) {
      errors.push(
        ...zodErrors(props.error, ["spec", "elements", elementId, "props"]),
      );
    } else {
      validateQueryBinding(
        parsed.data,
        elementId,
        element.type,
        props.data,
        errors,
      );
      validateLiveFields(
        parsed.data,
        elementId,
        element.type,
        props.data,
        objects,
        errors,
      );
    }

    if (!definition.allowsChildren && element.children.length > 0) {
      errors.push({
        path: `spec.elements.${elementId}.children`,
        message: `${element.type} does not accept children`,
      });
    }
  }

  validateGraph(parsed.data, errors);
  return errors.length > 0
    ? { success: false, errors }
    : { success: true, data: parsed.data };
}
