import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { activityDecisionPresentation, type ActivityDecision } from "@/lib/assistant/activity-decision";
import { useEffect, useState } from "react";
import { RECEIPT_MAX_W } from "@/lib/assistant/banner";
import { useMachinePreviewPolicy } from "@/hooks/use-machine-activity";
import { File, Globe, Terminal, Monitor } from "lucide-react";
import { ApiError } from "@/lib/api-client";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { ChatImage } from "@/lib/assistant/chat-types";
import type { MachineReceipt } from "@/schemas/machine-activity";
import { Button } from "@/components/ui/button";
import { ToolImage } from "./blocks/tool-image";

export function MachineToolCard({
  receipt,
  decision,
  conversationId,
  images = [],
}: {
  readonly decision?: ActivityDecision | null;
  readonly receipt: MachineReceipt;
  readonly conversationId?: string;
  readonly images?: readonly ChatImage[];
}) {
  useUiTranslation();
  const [preferenceOpen, setPreferenceOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [open, setOpen] = useState(false);
  const gated = activityDecisionPresentation(decision);
  const command = receipt.action.startsWith("command.");
  const Icon = command
    ? Terminal
    : receipt.action.startsWith("file.")
      ? File
      : receipt.action.startsWith("browser.")
        ? Globe
        : Monitor;
  const image = images.find((image) => image.id === receipt.screenshot_id);
  const root = conversationId
    ? `/assistant/nyxagent/conversations/${encodeURIComponent(conversationId)}`
    : undefined;
  const { query: policy, change } = useMachinePreviewPolicy(
    conversationId,
    command && preferenceOpen,
  );
  const enabled = policy.data?.enabled ?? false;
  async function changePolicy() {
    if (!root) return;
    setSaving(true);
    setError(undefined);
    try {
      await change(!enabled);
    } catch {
      setError("Could not change excerpt preference.");
    } finally {
      setSaving(false);
    }
  }
  return (
    <section
      aria-label={translateUi("Machine action")}
      className={`${RECEIPT_MAX_W} my-2 min-w-0 space-y-2 rounded-lg border border-hairline bg-overlay/35 p-3 text-13`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
        <span className="font-medium">
          {receipt.action.replaceAll(".", " · ").replaceAll("_", " ")}
        </span>
        <span
          role="status"
          className={
            (gated ? gated.status === "error" : receipt.status === "error")
              ? "text-destructive"
              : "text-muted-foreground"
          }
        >
          {gated?.label ?? receipt.status}
        </span>
        {receipt.exit_code !== null && <span>{translateUi("Exit ")}{receipt.exit_code}</span>}
        {receipt.bytes !== null && (
          <span>{receipt.bytes.toLocaleString(getFormattingLocale())}{translateUi(" bytes")}</span>
        )}
      </div>
      {gated ? <p className="text-muted-foreground">{gated.detail}</p> : null}
      <p className="break-all text-12 text-muted-foreground">
        {receipt.machine_name?.trim() ||
          translateUi("Machine {{value1}}", { value1: receipt.node_id.slice(0, 8) })}{" "}
        · {receipt.context_mode === "separated" ? translateUi("Separate workspace and browser for this agent") : receipt.context_mode === "unknown" ? translateUi("Workspace details unavailable") : translateUi("Shared workspace and browser")}
      </p>
      <details className="text-12 text-muted-foreground">
        <summary className="cursor-pointer">{translateUi("Correlation IDs")}</summary>
        <dl className="space-y-1 break-all pt-2">
          {[
            ["Machine", receipt.node_id],
            ["Agent", receipt.agent_id],
            ["Operation", receipt.operation_id],
            ["Job", receipt.job_id],
          ]
            .filter(([, value]) => value)
            .map(([label, value]) => (
              <div key={label}>
                <dt className="font-medium">{label}</dt>
                <dd>{value}</dd>
              </div>
            ))}
        </dl>
      </details>
      {(!gated || gated.status === "error") && receipt.error_code !== null && (
        <p role="alert">{translateUi("Machine error ")}{receipt.error_code}</p>
      )}
      {image && <ToolImage image={image} />}
      {receipt.preview_id && root && (
        <>
          <Button
            size="sm"
            variant="ghost"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            {open ? translateUi("Hide") : translateUi("Show")}{translateUi(" saved excerpt")}</Button>
          {open && (
            <CommandExcerpt
              key={receipt.preview_id}
              endpoint={`${root}/attachments/${encodeURIComponent(receipt.preview_id)}`}
            />
          )}
        </>
      )}
      {command && root && (
        <details
          className="text-muted-foreground"
          onToggle={(event) => setPreferenceOpen(event.currentTarget.open)}
        >
          <summary className="cursor-pointer">{translateUi("Command excerpt preference")}</summary>
          <p className="pt-2">{translateUi("Off by default. Save bounded, redacted output from future commands in this conversation for up to 30 days. Output can contain private data.")}</p>
          <Button
            size="sm"
            variant="outline"
            disabled={saving || !policy.data || policy.isFetching}
            onClick={() => void changePolicy()}
            className="mt-2"
          >
            {enabled ? translateUi("Stop saving excerpts") : translateUi("Save future excerpts")}
          </Button>
          {error && <p role="alert">{translateUi(error)}</p>}
          {policy.isError && (
            <p role="alert">{translateUi("Could not load excerpt preference.")}</p>
          )}
        </details>
      )}
    </section>
  );
}

function CommandExcerpt({ endpoint }: { readonly endpoint: string }) {
  useUiTranslation();
  const [text, setText] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [expired, setExpired] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    assistantHttp(endpoint, { signal: controller.signal })
      .then((response) => response.text())
      .then((value) => {
        if (!controller.signal.aborted) setText(value.slice(0, 4096));
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setFailed(true);
          setExpired(error instanceof ApiError && error.errorCode === 12101);
        }
      });
    return () => controller.abort();
  }, [endpoint]);
  if (expired)
    return <p role="status">{translateUi("Command excerpt expired per retention policy.")}</p>;
  if (failed) return <p role="status">{translateUi("Excerpt unavailable.")}</p>;
  if (text === undefined) return <p role="status">{translateUi("Loading excerpt…")}</p>;
  return (
    <pre data-scroll-fade="" className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-overlay p-2 font-mono text-14">
      {text}
    </pre>
  );
}
