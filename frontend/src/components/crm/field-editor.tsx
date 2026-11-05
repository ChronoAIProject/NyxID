import { useId } from "react";
import { CrmPeoplePicker } from "@/components/crm/people-picker";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { CrmRecordPicker } from "@/components/crm/record-picker";
import type { CrmField, CrmRecordLink } from "@/schemas/crm";

const UNSET = "__crm_unset__";

export function CrmFieldEditor({
  field,
  value,
  links,
  ownerId,
  error,
  disabled,
  onChange,
}: {
  readonly field: CrmField;
  readonly value: unknown;
  readonly links?: readonly CrmRecordLink[];
  readonly ownerId?: string;
  readonly error?: string;
  readonly disabled?: boolean;
  readonly onChange: (value: unknown) => void;
}) {
  const instanceId = useId();
  const id = `crm-field-${instanceId}-${field.key}`;
  const describedBy = error ? `${id}-error` : undefined;
  let control;

  if (
    field.type === "text" &&
    field.max_length > 254
  ) {
    control = (
      <Textarea
        id={id}
        value={typeof value === "string" ? value : ""}
        maxLength={field.max_length}
        disabled={disabled}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  } else if (field.type === "user") {
    control = (
      <CrmPeoplePicker
        id={id}
        describedBy={describedBy}
        value={value}
        ownerId={ownerId}
        disabled={disabled}
        onChange={onChange}
      />
    );
  } else if (field.type === "text") {
    control = (
      <Input
        id={id}
        value={typeof value === "string" ? value : ""}
        maxLength={field.type === "text" ? field.max_length : undefined}
        disabled={disabled}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  } else if (field.type === "number" || field.type === "currency") {
    control = (
      <div className="flex items-center gap-2">
        {field.type === "currency" ? (
          <span className="text-11 font-medium text-text-tertiary">
            {field.currency}
          </span>
        ) : null}
        <Input
          id={id}
          type="text"
          inputMode="decimal"
          value={
            typeof value === "string" || typeof value === "number" ? value : ""
          }
          disabled={disabled}
          aria-invalid={Boolean(error)}
          aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
        />
        {field.type === "number" && field.format === "percent" ? (
          <span className="text-12 text-text-tertiary">%</span>
        ) : null}
      </div>
    );
  } else if (field.type === "boolean") {
    control = field.required ? (
      <div className="flex min-h-8 items-center">
        <Switch
          id={id}
          checked={value === true}
          disabled={disabled}
          aria-describedby={describedBy}
          onCheckedChange={onChange}
        />
      </div>
    ) : (
      <Select
        value={
          value === null || value === undefined ? UNSET : String(value === true)
        }
        disabled={disabled}
        onValueChange={(next) =>
          onChange(next === UNSET ? null : next === "true")
        }
      >
        <SelectTrigger
          id={id}
          aria-invalid={Boolean(error)}
          aria-describedby={describedBy}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={UNSET}>Not set</SelectItem>
          <SelectItem value="true">Yes</SelectItem>
          <SelectItem value="false">No</SelectItem>
        </SelectContent>
      </Select>
    );
  } else if (field.type === "date" || field.type === "datetime") {
    control = (
      <Input
        id={id}
        type={field.type === "date" ? "date" : "datetime-local"}
        value={typeof value === "string" ? value : ""}
        disabled={disabled}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
    );
  } else if (field.type === "select") {
    const selected = typeof value === "string" && value ? value : UNSET;
    control = (
      <Select
        value={selected}
        disabled={disabled}
        onValueChange={(next) => onChange(next === UNSET ? null : next)}
      >
        <SelectTrigger
          id={id}
          aria-invalid={Boolean(error)}
          aria-describedby={describedBy}
        >
          <SelectValue placeholder="Select an option" />
        </SelectTrigger>
        <SelectContent>
          {!field.required ? <SelectItem value={UNSET}>None</SelectItem> : null}
          {field.options.map((option) => (
            <SelectItem
              key={option.value}
              value={option.value}
              disabled={option.archived && option.value !== value}
            >
              {option.label}
              {option.archived ? " (archived)" : ""}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  } else {
    control = (
      <CrmRecordPicker
        id={id}
        describedBy={describedBy}
        field={field}
        value={
          field.cardinality === "many"
            ? Array.isArray(value)
              ? value.filter((item): item is string => typeof item === "string")
              : []
            : typeof value === "string"
              ? value
              : null
        }
        selectedLinks={links}
        ownerId={ownerId}
        disabled={disabled}
        onChange={onChange}
      />
    );
  }

  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} className="text-12">
        {field.label}
        {field.required ? " *" : ""}
      </Label>
      {control}
      {error ? (
        <p id={`${id}-error`} role="alert" className="text-11 text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
