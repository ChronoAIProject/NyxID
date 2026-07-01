import { useState } from "react";
import type { ReactNode } from "react";
import { AlertTriangle, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
import { cn } from "@/lib/utils";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";
import type { ApiKeyCreateResponse } from "@/types/api";

const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const PROBE_TIMEOUT_MS = 8000;

/**
 * Phase machine — every transition is gated by an explicit user click
 * per Calvin's feedback ("the lift should be to ask them if they want
 * to create a key for this ai service instead of doing it
 * automatically. test comes after key is created"). Nothing fires in
 * the background.
 */
export type Phase =
  | "connected"       // step 2 is CURRENT
  | "minting"
  | "mint_failed"
  | "key_ready"       // step 3 is CURRENT
  | "probing"
  | "probe_success"   // step 4 is CURRENT
  | "probe_failed";

export interface CreatedKey {
  readonly id: string;
  readonly slug: string;
  readonly catalogSlug: string;
  readonly serviceName: string;
  readonly completionMode: "credential" | "device_code" | "oauth" | "none";
}

interface ConnectVerifyStepProps {
  readonly createdKey: CreatedKey;
  readonly isNodeRouted: boolean;
  readonly onDone: () => void;
}

function probePathForSlug(slug: string): string {
  return OPENAI_SHAPED_HINTS.test(slug) ? "v1/models" : "";
}

function proxyBaseUrl(slug: string): string {
  return `${window.location.origin}/api/v1/proxy/s/${slug}`;
}

type StepId = "connect" | "key" | "test" | "wire";

interface StepDef {
  readonly id: StepId;
  readonly index: number;
  readonly title: string;
  readonly summary: string;
}

const STEPS: readonly StepDef[] = [
  {
    id: "connect",
    index: 1,
    title: "Connect service",
    summary: "Credentials stored securely on NyxID.",
  },
  {
    id: "key",
    index: 2,
    title: "Create Agent Key",
    summary:
      "A scoped bearer your AI tools use to call this service through NyxID — without ever seeing your original credentials.",
  },
  {
    id: "test",
    index: 3,
    title: "Test the Agent Key",
    summary: "Fire one bearer-authenticated call to prove the path works.",
  },
  {
    id: "wire",
    index: 4,
    title: "Wire into your AI tool",
    summary: "Copy the Agent Key + Base URL into your tool's config.",
  },
] as const;

/**
 * Which step should carry the CURRENT focus (expanded body + action
 * buttons) based on the phase machine.
 */
function currentStep(phase: Phase): StepId {
  switch (phase) {
    case "connected":
    case "minting":
    case "mint_failed":
      return "key";
    case "key_ready":
    case "probing":
    case "probe_failed":
      return "test";
    case "probe_success":
      return "wire";
  }
}

/**
 * Which steps have been completed (rendered with a checkmark) based on
 * the phase machine. Order matters — earlier steps are considered done
 * once we've moved past them.
 */
function completedSteps(phase: Phase): ReadonlySet<StepId> {
  // Connect is always done — the verify step only mounts after the
  // POST /keys mutation returned successfully.
  const done = new Set<StepId>(["connect"]);
  if (phase === "key_ready" || phase === "probing" || phase === "probe_failed" || phase === "probe_success") {
    done.add("key");
  }
  if (phase === "probe_success") {
    done.add("test");
  }
  return done;
}

/**
 * Wave-aha-1 A4+ — umbrella "setup journey" layout per Calvin's
 * feedback: "this whole thing needs to be tied under a singular
 * umbrella to showcase what has happened and the logical next steps."
 *
 * Every step (Connect / Create Key / Test / Wire) is always visible.
 * Completed steps render with a checkmark and muted text. The current
 * step is expanded with its body content + action buttons. Future
 * steps show only their title + one-line summary in muted text.
 *
 * This gives the user the whole roadmap at a glance instead of a
 * single-phase modal where they can't tell how far they've come or
 * how much is left.
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  onDone,
}: ConnectVerifyStepProps) {
  void isNodeRouted; // referenced only in the diagnose() helper below

  const createApiKey = useCreateApiKey();
  const [phase, setPhase] = useState<Phase>("connected");
  const [agentKey, setAgentKey] = useState<ApiKeyCreateResponse | null>(null);
  const [mintError, setMintError] = useState<string | null>(null);
  const [httpStatus, setHttpStatus] = useState<number | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);

  function triggerMint() {
    console.info("[aha] triggerMint fired", createdKey.id, createdKey.serviceName);
    setPhase("minting");
    setMintError(null);
    createApiKey.mutate(
      {
        name: `Key for ${createdKey.serviceName}`,
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: [createdKey.id],
        allow_all_nodes: true,
      },
      {
        onSuccess: (key) => {
          console.info("[aha] mint success", key.key_prefix);
          setAgentKey(key);
          setPhase("key_ready");
        },
        onError: (err) => {
          const msg = err instanceof Error ? err.message : String(err);
          console.warn("[aha] mint failed", msg, err);
          setMintError(msg);
          setPhase("mint_failed");
        },
      },
    );
  }

  async function triggerProbe() {
    if (!agentKey) return;
    console.info("[aha] triggerProbe fired", createdKey.slug);
    setPhase("probing");
    setProbeError(null);
    setHttpStatus(null);

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));

    const url = `${proxyBaseUrl(createdKey.slug)}/${probePathForSlug(createdKey.slug)}`;
    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort(),
      PROBE_TIMEOUT_MS,
    );

    let status: number | null = null;
    try {
      const res = await fetch(url, {
        method: "GET",
        credentials: "omit",
        headers: {
          Authorization: `Bearer ${agentKey.full_key}`,
          "Content-Type": "application/json",
        },
        signal: controller.signal,
      });
      status = res.status;
    } catch {
      status = null;
    } finally {
      window.clearTimeout(timer);
    }

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
    console.info("[aha] probe returned HTTP", status);
    setHttpStatus(status);

    const ok = status !== null && status >= 200 && status < 400;
    if (ok) {
      setPhase("probe_success");
      window.dispatchEvent(
        new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT),
      );
    } else {
      setPhase("probe_failed");
      setProbeError(diagnose(status, isNodeRouted));
    }
  }

  const current = currentStep(phase);
  const done = completedSteps(phase);

  return (
    <div className="space-y-1 py-1">
      {STEPS.map((step) => (
        <StepRow
          key={step.id}
          step={step}
          isCurrent={step.id === current}
          isDone={done.has(step.id)}
        >
          {step.id === current && (
            <CurrentStepBody
              step={step}
              phase={phase}
              createdKey={createdKey}
              agentKey={agentKey}
              mintError={mintError}
              probeError={probeError}
              httpStatus={httpStatus}
              onMint={triggerMint}
              onProbe={triggerProbe}
              onDone={onDone}
            />
          )}
        </StepRow>
      ))}
    </div>
  );
}

function StepRow({
  step,
  isCurrent,
  isDone,
  children,
}: {
  readonly step: StepDef;
  readonly isCurrent: boolean;
  readonly isDone: boolean;
  readonly children?: ReactNode;
}) {
  return (
    <div
      className={cn(
        "rounded-xl border p-4",
        isCurrent
          ? "border-primary/40 bg-primary/5"
          : "border-border/40 bg-transparent",
      )}
    >
      <div className="flex items-start gap-3">
        <StepBadge step={step} isCurrent={isCurrent} isDone={isDone} />
        <div className="min-w-0 flex-1 space-y-0.5">
          <div className="flex items-center gap-2">
            <p
              className={cn(
                "text-[13px] font-semibold",
                isCurrent
                  ? "text-foreground"
                  : isDone
                    ? "text-muted-foreground line-through"
                    : "text-muted-foreground",
              )}
            >
              Step {step.index}. {step.title}
            </p>
            {isDone && !isCurrent && (
              <span className="text-[10px] uppercase tracking-wide text-success/80">
                Done
              </span>
            )}
            {isCurrent && (
              <span className="text-[10px] uppercase tracking-wide text-primary">
                Now
              </span>
            )}
          </div>
          <p
            className={cn(
              "text-[11px] leading-relaxed",
              isCurrent ? "text-muted-foreground" : "text-muted-foreground/70",
            )}
          >
            {step.summary}
          </p>
        </div>
      </div>
      {children && <div className="mt-3 pl-9">{children}</div>}
    </div>
  );
}

function StepBadge({
  step,
  isCurrent,
  isDone,
}: {
  readonly step: StepDef;
  readonly isCurrent: boolean;
  readonly isDone: boolean;
}) {
  if (isDone && !isCurrent) {
    return (
      <span className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-success/20 text-success">
        <Check className="h-3.5 w-3.5" aria-hidden />
      </span>
    );
  }
  if (isCurrent) {
    return (
      <span className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground text-[11px] font-semibold">
        {step.index}
      </span>
    );
  }
  return (
    <span className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-border text-[11px] text-muted-foreground/70">
      {step.index}
    </span>
  );
}

function CurrentStepBody({
  step,
  phase,
  createdKey,
  agentKey,
  mintError,
  probeError,
  httpStatus,
  onMint,
  onProbe,
  onDone,
}: {
  readonly step: StepDef;
  readonly phase: Phase;
  readonly createdKey: CreatedKey;
  readonly agentKey: ApiKeyCreateResponse | null;
  readonly mintError: string | null;
  readonly probeError: string | null;
  readonly httpStatus: number | null;
  readonly onMint: () => void;
  readonly onProbe: () => void;
  readonly onDone: () => void;
}) {
  if (step.id === "key") {
    return (
      <div className="space-y-3">
        {phase === "mint_failed" && (
          <InlineStatus
            tone="warn"
            icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
          >
            {mintError ??
              "The server didn't accept the request."}{" "}
            You can retry, or create one later at{" "}
            <span className="font-medium">/keys → Agent Keys → Create</span>.
          </InlineStatus>
        )}
        <div className="flex items-center justify-end gap-2">
          <Button variant="outline" size="lg" onClick={onDone}>
            Maybe later
          </Button>
          <Button
            variant="primary"
            size="lg"
            onClick={onMint}
            disabled={phase === "minting"}
            isLoading={phase === "minting"}
          >
            {phase === "mint_failed" ? "Try again" : "Create Agent Key"}
          </Button>
        </div>
      </div>
    );
  }

  if (step.id === "test") {
    return (
      <div className="space-y-3">
        {agentKey && (
          <AgentKeyPanel agentKey={agentKey} createdKey={createdKey} />
        )}
        {phase === "probe_failed" && (
          <InlineStatus
            tone="warn"
            icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
          >
            {probeError ??
              "The probe returned an unexpected response."}
            {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
            {" "}The Agent Key is still good — use it from your tool or retry.
          </InlineStatus>
        )}
        <div className="flex items-center justify-end gap-2">
          <Button variant="outline" size="lg" onClick={onDone}>
            {phase === "probe_failed" ? "Skip test" : "I'll wire it myself"}
          </Button>
          <Button
            variant="primary"
            size="lg"
            onClick={onProbe}
            disabled={phase === "probing"}
            isLoading={phase === "probing"}
          >
            {phase === "probe_failed" ? "Retry test" : "Test Agent Key"}
          </Button>
        </div>
      </div>
    );
  }

  // step.id === "wire"
  return (
    <div className="space-y-3">
      {agentKey && (
        <AgentKeyPanel
          agentKey={agentKey}
          createdKey={createdKey}
          showOpenAiEnvSnippet
        />
      )}
      <InlineStatus
        tone="success"
        icon={<Check className="h-4 w-4 text-success" aria-hidden />}
      >
        End-to-end verified
        {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}.
        Copy the Agent Key + Base URL above into your tool of choice — you&apos;re done.
      </InlineStatus>
      <div className="flex items-center justify-end gap-2">
        <Button variant="primary" size="lg" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

function InlineStatus({
  tone,
  icon,
  children,
}: {
  readonly tone: "neutral" | "success" | "warn";
  readonly icon: ReactNode;
  readonly children: ReactNode;
}) {
  const toneClass =
    tone === "success"
      ? "text-foreground"
      : tone === "warn"
        ? "text-foreground"
        : "text-muted-foreground";
  return (
    <div className="flex items-start gap-2">
      <span className="mt-0.5 shrink-0">{icon}</span>
      <p className={`text-[12px] leading-relaxed ${toneClass}`}>{children}</p>
    </div>
  );
}

function AgentKeyPanel({
  agentKey,
  createdKey,
  showOpenAiEnvSnippet = false,
}: {
  readonly agentKey: ApiKeyCreateResponse;
  readonly createdKey: CreatedKey;
  readonly showOpenAiEnvSnippet?: boolean;
}) {
  const proxyUrl = proxyBaseUrl(createdKey.slug);
  const isOpenAiShaped = OPENAI_SHAPED_HINTS.test(createdKey.slug);

  return (
    <div className="space-y-3 rounded-lg border border-border/50 bg-card p-3">
      <div className="space-y-1">
        <p className="text-[11px] font-semibold text-foreground">Agent Key</p>
        <p className="text-[11px] text-muted-foreground">
          Save this now — the full secret is only shown here once.
        </p>
      </div>
      <CopyableField label="Agent Key" value={agentKey.full_key} />
      <CopyableField label="Base URL" value={proxyUrl} />
      {showOpenAiEnvSnippet && isOpenAiShaped && (
        <>
          <p className="text-[11px] text-muted-foreground pt-1">
            Most OpenAI-compatible tools read these two env vars:
          </p>
          <CopyableField
            label="Shell env"
            value={`export OPENAI_API_KEY="${agentKey.full_key}"\nexport OPENAI_BASE_URL="${proxyUrl}/v1"`}
          />
        </>
      )}
    </div>
  );
}

function diagnose(
  status: number | null,
  isNodeRouted: boolean,
): string {
  if (status === null) {
    return "The probe timed out or was blocked by the browser.";
  }
  if (status === 401 || status === 403) {
    return `The downstream rejected the bearer-authenticated call (HTTP ${String(status)}). The upstream credential may have been mistyped or revoked.`;
  }
  if (status === 404) {
    if (isNodeRouted) {
      return "The proxy returned 404 — node-routed services need the node agent online to respond.";
    }
    return "The proxy returned 404 — the slug isn't routable yet, or the downstream doesn't expose this probe path.";
  }
  if (status === 429) {
    return "The downstream is rate-limiting (HTTP 429). Your credential + Agent Key work — retry in a minute.";
  }
  if (status >= 500) {
    return `The downstream returned a server error (HTTP ${String(status)}). NyxID's part of the path is fine; the upstream is having issues.`;
  }
  return `The probe returned an unexpected HTTP ${String(status)}.`;
}
