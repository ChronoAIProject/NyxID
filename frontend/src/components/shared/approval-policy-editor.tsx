import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  approvalVerbLabels,
  approvalVerbs,
} from "@/lib/approval-policies";
import type {
  ApprovalEffect,
  ApprovalMode,
  ApprovalRule,
  ApprovalVerb,
} from "@/types/approvals";

const approvalMethods = [
  "GET",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "HEAD",
  "OPTIONS",
  "EXEC",
  "TUNNEL",
] as const;

const effectLabels: Record<ApprovalEffect, string> = {
  require_approval: "Require approval",
  auto_allow: "Auto allow",
  deny: "Deny",
};

interface ApprovalPolicyEditorProps {
  readonly approvalMode: ApprovalMode;
  readonly defaultEffect: ApprovalEffect | null;
  readonly disabled?: boolean;
  readonly rules: readonly ApprovalRule[];
  readonly onSave: (
    rules: readonly ApprovalRule[],
    defaultEffect: ApprovalEffect,
  ) => void;
}

export function ApprovalPolicyEditor({
  approvalMode,
  defaultEffect,
  disabled = false,
  rules,
  onSave,
}: ApprovalPolicyEditorProps) {
  const [draftRules, setDraftRules] = useState<readonly ApprovalRule[]>(rules);
  const [draftDefaultEffect, setDraftDefaultEffect] =
    useState<ApprovalEffect>(defaultEffect ?? "auto_allow");

  function updateRule(index: number, update: Partial<ApprovalRule>) {
    setDraftRules((current) =>
      current.map((rule, idx) =>
        idx === index ? { ...rule, ...update } : rule,
      ),
    );
  }

  function toggleMethod(index: number, method: string, checked: boolean) {
    const current = draftRules[index];
    if (!current) return;
    const next = new Set(current.methods);
    if (checked) {
      next.add(method);
    } else {
      next.delete(method);
    }
    updateRule(index, { methods: Array.from(next) });
  }

  function toggleVerb(index: number, verb: ApprovalVerb, checked: boolean) {
    const current = draftRules[index];
    if (!current) return;
    const next = new Set(current.verbs);
    if (checked) {
      next.add(verb);
    } else {
      next.delete(verb);
    }
    updateRule(index, { verbs: approvalVerbs.filter((v) => next.has(v)) });
  }

  function addRule() {
    setDraftRules((current) => [
      ...current,
      {
        methods: [],
        resource_pattern: "*",
        verbs: [],
        effect: "require_approval",
        mode: approvalMode,
      },
    ]);
  }

  function removeRule(index: number) {
    setDraftRules((current) => current.filter((_, idx) => idx !== index));
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-muted-foreground">Default:</span>
        <Select
          value={draftDefaultEffect}
          onValueChange={(value) =>
            setDraftDefaultEffect(value as ApprovalEffect)
          }
        >
          <SelectTrigger className="h-8 w-[190px] text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="auto_allow">Auto allow unmatched</SelectItem>
            <SelectItem value="require_approval">
              Require approval unmatched
            </SelectItem>
            <SelectItem value="deny">Deny unmatched</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {draftRules.length === 0 ? (
        <div className="rounded-lg bg-white/[0.03] px-3 py-2 text-xs text-muted-foreground">
          No advanced rules configured.
        </div>
      ) : (
        <div className="space-y-3">
          {draftRules.map((rule, index) => (
            <div
              key={index}
              className="space-y-3 rounded-lg border border-border p-3"
            >
              <div className="flex items-center justify-between gap-3">
                <span className="text-[12px] font-medium">
                  Rule {index + 1}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => removeRule(index)}
                  disabled={disabled}
                  title="Remove rule"
                >
                  <Trash2 className="h-4 w-4 text-muted-foreground" />
                </Button>
              </div>

              <div className="space-y-2">
                <span className="text-xs text-muted-foreground">Methods</span>
                <div className="flex flex-wrap gap-2">
                  {approvalMethods.map((method) => (
                    <label
                      key={method}
                      className="flex items-center gap-2 text-xs text-muted-foreground"
                    >
                      <Checkbox
                        checked={rule.methods.includes(method)}
                        onCheckedChange={(checked) =>
                          toggleMethod(index, method, checked === true)
                        }
                        disabled={disabled}
                      />
                      {method}
                    </label>
                  ))}
                </div>
              </div>

              <div className="space-y-2">
                <label className="text-xs text-muted-foreground">
                  Resource pattern
                </label>
                <Input
                  value={rule.resource_pattern}
                  onChange={(event) =>
                    updateRule(index, {
                      resource_pattern: event.target.value,
                    })
                  }
                  maxLength={256}
                  disabled={disabled}
                />
              </div>

              <div className="flex flex-wrap gap-3">
                {approvalVerbs.map((verb) => (
                  <label
                    key={verb}
                    className="flex items-center gap-2 text-xs text-muted-foreground"
                  >
                    <Checkbox
                      checked={rule.verbs.includes(verb)}
                      onCheckedChange={(checked) =>
                        toggleVerb(index, verb, checked === true)
                      }
                      disabled={disabled}
                    />
                    {approvalVerbLabels[verb]}
                  </label>
                ))}
              </div>

              <div className="flex flex-wrap gap-2">
                <Select
                  value={rule.effect}
                  onValueChange={(value) =>
                    updateRule(index, { effect: value as ApprovalEffect })
                  }
                >
                  <SelectTrigger className="h-8 w-[170px] text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {Object.entries(effectLabels).map(([value, label]) => (
                      <SelectItem key={value} value={value}>
                        {label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {rule.effect === "require_approval" && (
                  <Select
                    value={rule.mode}
                    onValueChange={(value) =>
                      updateRule(index, { mode: value as ApprovalMode })
                    }
                  >
                    <SelectTrigger className="h-8 w-[170px] text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="per_request">Per request</SelectItem>
                      <SelectItem value="grant">Time-based grant</SelectItem>
                    </SelectContent>
                  </Select>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          className="h-8 text-xs"
          onClick={addRule}
          disabled={disabled || draftRules.length >= 50}
        >
          <ButtonIcon>
            <Plus className="h-3.5 w-3.5" />
          </ButtonIcon>
          Add Rule
        </Button>
        <Button
          variant="primary"
          className="h-8 text-xs"
          onClick={() => onSave(draftRules, draftDefaultEffect)}
          disabled={disabled}
        >
          Save Rules
        </Button>
      </div>
    </div>
  );
}
