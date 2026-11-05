import { CrmPeoplePicker } from "@/components/crm/people-picker";
import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CrmRecordPicker } from "@/components/crm/record-picker";
import {
  CRM_FILTER_OPERATOR_LABELS,
  crmFilterOperators,
  formatCrmFieldInput,
  parseCrmFieldInput,
} from "@/lib/crm/field-kinds";
import type {
  CrmField,
  CrmFilter,
  CrmFilterOperator,
  CrmObjectMetadata,
} from "@/schemas/crm";

function needsValue(operator: CrmFilterOperator) {
  return operator !== "is_set" && operator !== "is_unset";
}

function defaultOperator(field: CrmField): CrmFilterOperator {
  return crmFilterOperators(field)[0] ?? "eq";
}

function FilterValue({
  field,
  filter,
  ownerId,
  onChange,
}: {
  readonly field: CrmField;
  readonly filter: CrmFilter;
  readonly ownerId?: string;
  readonly onChange: (value: unknown) => void;
}) {
  const [error, setError] = useState<string>();
  const [draft, setDraft] = useState(() => {
    if (Array.isArray(filter.value)) return filter.value.join(", ");
    const formatted = formatCrmFieldInput(field, filter.value);
    return typeof formatted === "string" || typeof formatted === "number"
      ? String(formatted)
      : "";
  });
  if (!needsValue(filter.op)) return null;
  if (field.type === "boolean") {
    return (
      <Select
        value={filter.value === true ? "true" : "false"}
        onValueChange={(value) => onChange(value === "true")}
      >
        <SelectTrigger aria-label={`${field.label} value`}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="true">Yes</SelectItem>
          <SelectItem value="false">No</SelectItem>
        </SelectContent>
      </Select>
    );
  }
  if (field.type === "select" && filter.op !== "in" && filter.op !== "nin") {
    return (
      <Select
        value={typeof filter.value === "string" ? filter.value : ""}
        onValueChange={onChange}
      >
        <SelectTrigger aria-label={`${field.label} value`}>
          <SelectValue placeholder="Choose value" />
        </SelectTrigger>
        <SelectContent>
          {field.options
            .filter((option) => !option.archived)
            .map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
        </SelectContent>
      </Select>
    );
  }
  if (field.type === "user" && filter.op !== "in" && filter.op !== "nin")
    return (
      <CrmPeoplePicker
        value={filter.value}
        ownerId={ownerId}
        onChange={onChange}
      />
    );
  if (field.type === "record") {
    const many = filter.op === "in";
    return (
      <CrmRecordPicker
        field={{ ...field, cardinality: many ? "many" : "one" }}
        value={
          many
            ? Array.isArray(filter.value)
              ? filter.value.filter(
                  (value): value is string => typeof value === "string",
                )
              : []
            : typeof filter.value === "string"
              ? filter.value
              : null
        }
        ownerId={ownerId}
        onChange={onChange}
      />
    );
  }
  return (
    <div className="space-y-1">
      <Input
        aria-invalid={Boolean(error)}
        aria-label={`${field.label} value`}
        type={
          field.type === "date"
            ? "date"
            : field.type === "datetime"
              ? "datetime-local"
              : "text"
        }
        inputMode={
          field.type === "number" || field.type === "currency"
            ? "decimal"
            : undefined
        }
        maxLength={filter.op === "contains" ? 100 : undefined}
        value={draft}
        onChange={(event) => {
          const next = event.target.value;
          setDraft(next);
          try {
            setError(undefined);
            onChange(
              filter.op === "in" || filter.op === "nin"
                ? next
                    .split(",")
                    .map((value) => value.trim())
                    .filter(Boolean)
                    .slice(0, 100)
                : parseCrmFieldInput(field, next),
            );
          } catch (cause) {
            setError(
              `${cause instanceof Error ? cause.message : "Invalid value"} The previous valid filter is still active.`,
            );
          }
        }}
        placeholder={
          filter.op === "in" || filter.op === "nin"
            ? "Comma-separated values"
            : undefined
        }
      />
      {error ? (
        <p role="alert" className="text-11 text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}

export function CrmFilterControls({
  object,
  filters,
  ownerId,
  onChange,
}: {
  readonly object: CrmObjectMetadata;
  readonly filters: readonly CrmFilter[];
  readonly ownerId?: string;
  readonly onChange: (filters: CrmFilter[]) => void;
}) {
  const availableFields = object.fields.filter((field) => !field.archived);
  function addFilter(fieldKey: string) {
    const field = availableFields.find(
      (candidate) => candidate.key === fieldKey,
    );
    if (!field || filters.length >= 20) return;
    const op = defaultOperator(field);
    onChange([
      ...filters,
      {
        field: field.key,
        op,
        value: field.type === "boolean" ? true : undefined,
      },
    ]);
  }
  return (
    <div className="space-y-2" aria-label="Filters">
      {filters.map((filter, index) => {
        const field = availableFields.find(
          (candidate) => candidate.key === filter.field,
        );
        if (!field) return null;
        return (
          <div
            key={`${filter.field}-${index}`}
            className="grid gap-2 rounded-lg border border-border/50 bg-card p-2 sm:grid-cols-[minmax(8rem,1fr)_minmax(8rem,1fr)_minmax(10rem,2fr)_auto]"
          >
            <Select
              value={filter.field}
              onValueChange={(fieldKey) => {
                const nextField = availableFields.find(
                  (candidate) => candidate.key === fieldKey,
                );
                if (!nextField) return;
                const next = [...filters];
                next[index] = {
                  field: fieldKey,
                  op: defaultOperator(nextField),
                };
                onChange(next);
              }}
            >
              <SelectTrigger aria-label={`Filter ${index + 1} field`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {availableFields.map((candidate) => (
                  <SelectItem key={candidate.key} value={candidate.key}>
                    {candidate.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={filter.op}
              onValueChange={(operator: CrmFilterOperator) => {
                const next = [...filters];
                next[index] = {
                  field: filter.field,
                  op: operator,
                  value: needsValue(operator) ? filter.value : undefined,
                };
                onChange(next);
              }}
            >
              <SelectTrigger aria-label={`Filter ${index + 1} operator`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {crmFilterOperators(field).map((operator) => (
                  <SelectItem key={operator} value={operator}>
                    {CRM_FILTER_OPERATOR_LABELS[operator]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FilterValue
              field={field}
              filter={filter}
              ownerId={ownerId}
              onChange={(value) => {
                const next = [...filters];
                next[index] = { ...filter, value };
                onChange(next);
              }}
            />
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={`Remove ${field.label} filter`}
              onClick={() =>
                onChange(filters.filter((_, candidate) => candidate !== index))
              }
            >
              <Trash2 />
            </Button>
          </div>
        );
      })}
      {filters.length < 20 ? (
        <Select onValueChange={addFilter}>
          <SelectTrigger className="w-36" aria-label="Add filter">
            <span className="inline-flex items-center gap-1.5">
              <Plus />
              Add filter
            </span>
          </SelectTrigger>
          <SelectContent>
            {availableFields.map((field) => (
              <SelectItem key={field.key} value={field.key}>
                {field.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
    </div>
  );
}
