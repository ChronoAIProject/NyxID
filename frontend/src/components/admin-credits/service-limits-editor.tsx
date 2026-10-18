import { useEffect, useId, useRef, useState } from "react";
import type { FieldErrors } from "react-hook-form";
import type { AddCreditsForm } from "@/schemas/billing-credits";
import type { DownstreamService } from "@/types/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ServicePicker } from "./credit-pickers";

type Limit = NonNullable<AddCreditsForm["service_limits"]>[number];

export function ServiceLimitsEditor({
  rules,
  onChange,
  errors,
  services,
  loading,
  error,
  onRetry,
  disabled,
}: {
  readonly rules: readonly Limit[];
  readonly onChange: (rules: Limit[]) => void;
  readonly errors?: FieldErrors<{ service_limits: Limit[] }>["service_limits"];
  readonly services: readonly DownstreamService[];
  readonly loading: boolean;
  readonly error: boolean;
  readonly onRetry: () => void;
  readonly disabled: boolean;
}) {
  const id = useId();
  const outer = useRef<HTMLDetailsElement>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const invalid = rules.findIndex((_, index) => Boolean(errors?.[index]));
  const openIndex = invalid >= 0 ? invalid : expanded;
  useEffect(() => {
    if (invalid >= 0) {
      if (outer.current) outer.current.open = true;
      const frame = requestAnimationFrame(() => {
        outer.current
          ?.querySelector<HTMLInputElement>('[aria-invalid="true"]')
          ?.focus();
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [invalid]);
  function update(index: number, patch: Partial<Limit>) {
    onChange(
      rules.map((rule, i) => (i === index ? { ...rule, ...patch } : rule)),
    );
  }
  return (
    <details open ref={outer} className="rounded-lg border border-border">
      <summary className="cursor-pointer px-3 py-2 text-[12px] font-medium">
        Service limits (optional)
        {rules.length ? ` · ${String(rules.length)}` : ""}
      </summary>
      <fieldset
        disabled={disabled}
        className="min-w-0 space-y-4 border-t border-border p-3"
      >
        <p className="text-[11px] text-muted-foreground">
          Limits share the allocation’s total; they do not set aside or add
          credits. Other allocations and purchased credits can cover additional
          usage. With no limits, eligible services share the total.
        </p>
        {rules.map((rule, index) => {
          const prefix = `${id}-${String(index)}`;
          const fieldErrors = errors?.[index];
          return (
            <details
              key={prefix}
              open={openIndex === index}
              className="min-w-0 rounded-lg border border-border"
            >
              <summary
                className="cursor-pointer [overflow-wrap:anywhere] px-3 py-2 text-xs"
                onClick={(event) => {
                  event.preventDefault();
                  setExpanded((current) => (current === index ? null : index));
                }}
              >
                {rule.label || `Limit ${String(index + 1)}`} ·{" "}
                {rule.service_refs.length
                  ? rule.service_refs
                      .map(
                        (ref) =>
                          services.find((service) => service.id === ref)
                            ?.name ?? ref,
                      )
                      .join(", ")
                  : "Choose services"}{" "}
                · up to{" "}
                {Number.isFinite(rule.amount_credits)
                  ? new Intl.NumberFormat().format(rule.amount_credits)
                  : "—"}{" "}
                credits
              </summary>
              <fieldset className="min-w-0 space-y-3 border-t border-border p-3">
                <legend className="sr-only">Edit limit {index + 1}</legend>
                <div className="grid min-w-0 gap-3 sm:grid-cols-2">
                  <div className="min-w-0 space-y-1">
                    <label htmlFor={`${prefix}-label`} className="text-xs">
                      Group label (optional)
                    </label>
                    <Input
                      id={`${prefix}-label`}
                      value={rule.label}
                      maxLength={120}
                      onChange={(e) => update(index, { label: e.target.value })}
                      aria-invalid={!!fieldErrors?.label}
                      aria-describedby={
                        fieldErrors?.label ? `${prefix}-label-error` : undefined
                      }
                    />
                    {fieldErrors?.label ? (
                      <p
                        id={`${prefix}-label-error`}
                        className="text-xs text-destructive"
                      >
                        {fieldErrors.label.message}
                      </p>
                    ) : null}
                  </div>
                  <div className="min-w-0 space-y-1">
                    <label htmlFor={`${prefix}-amount`} className="text-xs">
                      Credit limit
                    </label>
                    <Input
                      id={`${prefix}-amount`}
                      type="number"
                      min={1}
                      max={1_000_000}
                      step={1}
                      value={
                        Number.isFinite(rule.amount_credits)
                          ? rule.amount_credits
                          : ""
                      }
                      onChange={(e) =>
                        update(index, {
                          amount_credits: e.target.valueAsNumber,
                        })
                      }
                      aria-invalid={!!fieldErrors?.amount_credits}
                      aria-describedby={
                        fieldErrors?.amount_credits
                          ? `${prefix}-amount-error`
                          : undefined
                      }
                    />
                    {fieldErrors?.amount_credits ? (
                      <p
                        id={`${prefix}-amount-error`}
                        className="text-xs text-destructive"
                      >
                        {fieldErrors.amount_credits.message}
                      </p>
                    ) : null}
                  </div>
                </div>
                <div
                  role="group"
                  aria-label={`Services for limit ${String(index + 1)}`}
                  aria-describedby={
                    fieldErrors?.service_refs
                      ? `${prefix}-services-error`
                      : undefined
                  }
                >
                  <p className="mb-1 text-xs">Services sharing this limit</p>
                  <ServicePicker
                    services={services}
                    selected={rule.service_refs}
                    onChange={(ids) => update(index, { service_refs: ids })}
                    multiple
                    showMetric={false}
                    loading={loading}
                    error={error}
                    onRetry={onRetry}
                  />
                  {fieldErrors?.service_refs ? (
                    <p
                      id={`${prefix}-services-error`}
                      className="mt-1 text-xs text-destructive"
                    >
                      {fieldErrors.service_refs.message}
                    </p>
                  ) : null}
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={`Remove limit ${String(index + 1)}`}
                  onClick={() => onChange(rules.filter((_, i) => i !== index))}
                >
                  Remove limit
                </Button>
              </fieldset>
            </details>
          );
        })}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={rules.length >= 20}
          onClick={() => {
            setExpanded(rules.length);
            onChange([
              ...rules,
              { service_refs: [], amount_credits: 100, label: "" },
            ]);
          }}
        >
          Add service limit
        </Button>
        {errors?.message ? (
          <p className="text-xs text-destructive">{errors.message}</p>
        ) : null}
      </fieldset>
    </details>
  );
}
