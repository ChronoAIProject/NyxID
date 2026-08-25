import {
  AGUIEventType,
  parseCustomEvent,
  type AGUIEvent,
  type CustomEventName,
  type HumanInputRequestData,
  type RunContextData,
  type StepCompletedData,
  type StepRequestData,
  type WaitingSignalData,
} from "@/lib/assistant/agui-types";

type JsonRecord = Record<string, unknown>;

type CustomEventParser<T> = (value: unknown) => T | undefined;
export type RuntimeRunContextData = RunContextData & {
  readonly correlationId?: string;
};

function asRecord(value: unknown): JsonRecord | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }

  return value as JsonRecord;
}

function readOptionalString(
  record: JsonRecord,
  key: string,
): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function readOptionalBoolean(
  record: JsonRecord,
  key: string,
): boolean | undefined {
  const value = record[key];
  return typeof value === "boolean" ? value : undefined;
}

function readOptionalNumber(
  record: JsonRecord,
  key: string,
): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function readOptionalStringRecord(
  record: JsonRecord,
  key: string,
): Record<string, string> | undefined {
  const value = asRecord(record[key]);
  if (!value) {
    return undefined;
  }

  const entries = Object.entries(value);
  if (entries.some(([, entry]) => typeof entry !== "string")) {
    return undefined;
  }

  return Object.fromEntries(entries) as Record<string, string>;
}

function hasDefinedValues(values: unknown[]): boolean {
  return values.some((value) => value !== undefined);
}

export function parseRunContextData(
  value: unknown,
): RuntimeRunContextData | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }

  const data: RuntimeRunContextData = {
    actorId: readOptionalString(record, "actorId"),
    workflowName: readOptionalString(record, "workflowName"),
    commandId: readOptionalString(record, "commandId"),
    correlationId: readOptionalString(record, "correlationId"),
  };

  return hasDefinedValues(Object.values(data)) ? data : undefined;
}

export function parseStepRequestData(
  value: unknown,
): StepRequestData | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }

  const data: StepRequestData = {
    runId: readOptionalString(record, "runId"),
    stepId: readOptionalString(record, "stepId"),
    stepType: readOptionalString(record, "stepType"),
    input: readOptionalString(record, "input"),
    targetRole: readOptionalString(record, "targetRole"),
  };

  return hasDefinedValues(Object.values(data)) ? data : undefined;
}

export function parseStepCompletedData(
  value: unknown,
): StepCompletedData | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }

  const data: StepCompletedData = {
    runId: readOptionalString(record, "runId"),
    stepId: readOptionalString(record, "stepId"),
    success: readOptionalBoolean(record, "success"),
    output: readOptionalString(record, "output"),
    error: readOptionalString(record, "error"),
  };

  return hasDefinedValues(Object.values(data)) ? data : undefined;
}

export function parseHumanInputRequestData(
  value: unknown,
): HumanInputRequestData | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }

  const data: HumanInputRequestData = {
    runId: readOptionalString(record, "runId"),
    stepId: readOptionalString(record, "stepId"),
    suspensionType: readOptionalString(record, "suspensionType"),
    prompt: readOptionalString(record, "prompt"),
    timeoutSeconds: readOptionalNumber(record, "timeoutSeconds"),
    metadata: readOptionalStringRecord(record, "metadata"),
  };

  return hasDefinedValues(Object.values(data)) ? data : undefined;
}

export function parseWaitingSignalData(
  value: unknown,
): WaitingSignalData | undefined {
  const record = asRecord(value);
  if (!record) {
    return undefined;
  }

  const data: WaitingSignalData = {
    runId: readOptionalString(record, "runId"),
    stepId: readOptionalString(record, "stepId"),
    signalName: readOptionalString(record, "signalName"),
    prompt: readOptionalString(record, "prompt"),
    timeoutMs: readOptionalNumber(record, "timeoutMs"),
  };

  return hasDefinedValues(Object.values(data)) ? data : undefined;
}

export function getLatestCustomEventData<T>(
  events: readonly AGUIEvent[],
  name: CustomEventName,
  parser: CustomEventParser<T>,
): T | undefined {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (!event || event.type !== AGUIEventType.CUSTOM) {
      continue;
    }

    const parsed = parseCustomEvent(event);
    if (parsed.name !== name) {
      continue;
    }

    const data = parser(parsed.data);
    if (data) {
      return data;
    }
  }

  return undefined;
}
