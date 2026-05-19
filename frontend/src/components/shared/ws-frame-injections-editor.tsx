import {
  type WsFrameInjection,
  type WsFrameTrigger,
} from "@/schemas/services";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plus, Trash2, Wand2 } from "lucide-react";

type WsFrameTriggerType =
  | "first_frame_from_downstream"
  | "json_field_equals"
  | "frame_index_from_downstream";

const emptyWsFrameRule: WsFrameInjection = {
  trigger: "first_frame_from_downstream",
  template: "",
  frame_kind: "text",
  consume_trigger: true,
  direction: "downstream",
};

const homeAssistantWsFrameRule: WsFrameInjection = {
  trigger: {
    json_field_equals: {
      path: "$.type",
      value: "auth_required",
    },
  },
  template: '{"type":"auth","access_token":"${credential}"}',
  frame_kind: "text",
  consume_trigger: true,
  direction: "downstream",
};

function cloneRule(rule: WsFrameInjection): WsFrameInjection {
  const trigger = rule.trigger;
  return {
    ...rule,
    trigger:
      typeof trigger === "object" && "json_field_equals" in trigger
        ? {
            json_field_equals: {
              path: trigger.json_field_equals.path,
              value: trigger.json_field_equals.value,
            },
          }
        : typeof trigger === "object" && "frame_index_from_downstream" in trigger
          ? {
              frame_index_from_downstream: {
                index: trigger.frame_index_from_downstream.index,
              },
            }
          : trigger,
  };
}

function wsTriggerType(trigger: WsFrameTrigger): WsFrameTriggerType {
  if (trigger === "first_frame_from_downstream") {
    return "first_frame_from_downstream";
  }
  if ("json_field_equals" in trigger) {
    return "json_field_equals";
  }
  return "frame_index_from_downstream";
}

function wsTriggerForType(type: WsFrameTriggerType): WsFrameTrigger {
  if (type === "json_field_equals") {
    return {
      json_field_equals: {
        path: "$.type",
        value: "auth_required",
      },
    };
  }
  if (type === "frame_index_from_downstream") {
    return { frame_index_from_downstream: { index: 0 } };
  }
  return "first_frame_from_downstream";
}

function formatTriggerValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined) return "";
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function parseTriggerValue(value: string): unknown {
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
}

interface WsFrameInjectionsEditorProps {
  readonly value: WsFrameInjection[];
  readonly onChange: (next: WsFrameInjection[]) => void;
  readonly errorMessage?: string;
}

export function WsFrameInjectionsEditor({
  value,
  onChange,
  errorMessage,
}: WsFrameInjectionsEditorProps) {
  const updateRule = (index: number, patch: Partial<WsFrameInjection>) => {
    onChange(
      value.map((rule, idx) => (idx === index ? { ...rule, ...patch } : rule)),
    );
  };

  return (
    <details className="rounded-xl border border-border/50 bg-card p-4">
      <summary className="cursor-pointer text-[13px] font-semibold text-foreground">
        WebSocket auth frames
      </summary>
      <div className="mt-4 space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={value.length >= 4}
            onClick={() => onChange([...value, cloneRule(emptyWsFrameRule)])}
          >
            <Plus />
            Add Rule
          </Button>
          <Button
            type="button"
            variant="secondary"
            onClick={() => onChange([cloneRule(homeAssistantWsFrameRule)])}
          >
            <Wand2 />
            Home Assistant Preset
          </Button>
          <Badge variant="secondary">{value.length}/4</Badge>
        </div>

        {errorMessage && (
          <p className="text-xs text-destructive">{errorMessage}</p>
        )}

        {value.map((rule, index) => {
          const triggerType = wsTriggerType(rule.trigger);
          const trigger = rule.trigger;
          const jsonField =
            typeof trigger === "object" &&
            trigger !== null &&
            "json_field_equals" in trigger
              ? trigger.json_field_equals
              : { path: "$.type", value: "auth_required" };
          const frameIndex =
            typeof trigger === "object" &&
            trigger !== null &&
            "frame_index_from_downstream" in trigger
              ? trigger.frame_index_from_downstream.index
              : 0;

          return (
            <div
              key={index}
              className="space-y-3 rounded-xl border border-border/50 bg-card p-4"
            >
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs font-medium">Rule {index + 1}</p>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove WebSocket auth frame rule ${index + 1}`}
                  onClick={() =>
                    onChange(value.filter((_, idx) => idx !== index))
                  }
                >
                  <Trash2 className="text-destructive" />
                </Button>
              </div>

              <div className="grid gap-3 sm:grid-cols-3">
                <div className="space-y-1.5">
                  <Label>Trigger</Label>
                  <Select
                    value={triggerType}
                    onValueChange={(next) =>
                      updateRule(index, {
                        trigger: wsTriggerForType(next as WsFrameTriggerType),
                      })
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="first_frame_from_downstream">
                        First downstream frame
                      </SelectItem>
                      <SelectItem value="json_field_equals">
                        JSON field equals
                      </SelectItem>
                      <SelectItem value="frame_index_from_downstream">
                        Downstream frame index
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label>Trigger direction</Label>
                  <Select
                    value={rule.direction}
                    onValueChange={(next) =>
                      updateRule(index, {
                        direction: next as WsFrameInjection["direction"],
                      })
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="downstream">From service</SelectItem>
                      <SelectItem value="upstream">From client</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label>Frame kind</Label>
                  <Select
                    value={rule.frame_kind}
                    onValueChange={(next) =>
                      updateRule(index, {
                        frame_kind: next as WsFrameInjection["frame_kind"],
                      })
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="text">Text</SelectItem>
                      <SelectItem value="binary">Binary</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {triggerType === "json_field_equals" && (
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-1.5">
                    <Label>JSON path</Label>
                    <Input
                      value={jsonField.path}
                      placeholder="$.type"
                      onChange={(event) =>
                        updateRule(index, {
                          trigger: {
                            json_field_equals: {
                              ...jsonField,
                              path: event.target.value,
                            },
                          },
                        })
                      }
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label>Expected value</Label>
                    <Input
                      value={formatTriggerValue(jsonField.value)}
                      placeholder="auth_required"
                      onChange={(event) =>
                        updateRule(index, {
                          trigger: {
                            json_field_equals: {
                              ...jsonField,
                              value: parseTriggerValue(event.target.value),
                            },
                          },
                        })
                      }
                    />
                  </div>
                </div>
              )}

              {triggerType === "frame_index_from_downstream" && (
                <div className="space-y-1.5">
                  <Label>Downstream frame index</Label>
                  <Input
                    type="number"
                    min={0}
                    value={frameIndex}
                    onChange={(event) =>
                      updateRule(index, {
                        trigger: {
                          frame_index_from_downstream: {
                            index: Math.max(
                              0,
                              Number(event.target.value) || 0,
                            ),
                          },
                        },
                      })
                    }
                  />
                </div>
              )}

              <div className="flex items-center gap-2">
                <Checkbox
                  id={`ws-consume-trigger-${index}`}
                  checked={rule.consume_trigger}
                  onCheckedChange={(checked) =>
                    updateRule(index, {
                      consume_trigger: checked === true,
                    })
                  }
                />
                <Label
                  htmlFor={`ws-consume-trigger-${index}`}
                  className="text-[12px] font-normal"
                >
                  Consume trigger frame
                </Label>
              </div>

              <div className="space-y-1.5">
                <Label>Injected frame template</Label>
                <textarea
                  className="flex min-h-[96px] w-full rounded-lg border border-input bg-transparent px-3 py-2 font-mono text-xs placeholder:text-muted-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
                  value={rule.template}
                  maxLength={4096}
                  placeholder='{"type":"auth","access_token":"${credential}"}'
                  onChange={(event) =>
                    updateRule(index, {
                      template: event.target.value,
                    })
                  }
                />
              </div>
            </div>
          );
        })}
      </div>
    </details>
  );
}
