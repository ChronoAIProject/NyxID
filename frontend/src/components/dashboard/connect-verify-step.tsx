import { useState } from "react";
import type { ReactNode } from "react";
import { AlertTriangle, Check, KeyRound, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
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
  | "connected"
  | "minting"
  | "mint_failed"
  | "key_ready"
  | "probing"
  | "probe_success"
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

/**
 * Wave-aha-1 A4+ — umbrella-view post-connect setup.
 *
 * The DialogTitle owns the umbrella state ("<service> connected").
 * This component's body morphs per phase:
 *
 *   connected      → one-line "you need an Agent Key to use this"
 *                    + Maybe later / Create Agent Key
 *   minting        → inline spinner
 *   mint_failed    → inline error + Maybe later / Try again
 *   key_ready      → Agent Key panel + I'll wire / Test Agent Key
 *   probing        → panel stays visible + inline spinner
 *   probe_success  → panel + env snippet + inline confirmation + Done
 *   probe_failed   → panel + inline error + Skip / Retry test
 *
 * No stepper, no repeated "connected" banners — the DialogTitle carries
 * that state exactly once, and everything else is either the current
 * action or its output.
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  onDone,
}: ConnectVerifyStepProps) {
  void isNodeRouted; // referenced only in diagnose() below

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
    const timer = window.setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);

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
      window.dispatchEvent(new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT));
    } else {
      setPhase("probe_failed");
      setProbeError(diagnose(status, isNodeRouted));
    }
  }

  return (
    <div className="space-y-4 py-2">
      <Body
        phase={phase}
        createdKey={createdKey}
        agentKey={agentKey}
        mintError={mintError}
        probeError={probeError}
        httpStatus={httpStatus}
      />
      <Footer
        phase={phase}
        onMint={triggerMint}
        onProbe={triggerProbe}
        onDone={onDone}
      />
    </div>
  );
}

function Body({
  phase,
  createdKey,
  agentKey,
  mintError,
  probeError,
  httpStatus,
}: {
  readonly phase: Phase;
  readonly createdKey: CreatedKey;
  readonly agentKey: ApiKeyCreateResponse | null;
  readonly mintError: string | null;
  readonly probeError: string | null;
  readonly httpStatus: number | null;
}) {
  if (phase === "connected") {
    // Umbrella-view "what next" line — the DialogTitle already says
    // "<service> connected"; this explains what an Agent Key is FOR so
    // the user knows what they're about to opt into.
    return (
      <div className="flex items-start gap-3 rounded-xl border border-border/50 bg-card p-4">
        <KeyRound
          className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <p className="text-[12px] leading-relaxed text-muted-foreground">
          Your AI tools need an{" "}
          <span className="font-medium text-foreground">Agent Key</span> to
          call <span className="font-medium text-foreground">{createdKey.serviceName}</span>{" "}
          through NyxID. NyxID injects your stored credentials server-side —
          your tool never sees the original secret.
        </p>
      </div>
    );
  }

  if (phase === "minting") {
    return (
      <InlineStatus icon={<Spinner />}>
        Creating your Agent Key…
      </InlineStatus>
    );
  }

  if (phase === "mint_failed") {
    return (
      <InlineStatus
        tone="warn"
        icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
      >
        {mintError ?? "The server didn't accept the request."}{" "}
        Retry, or create one later at{" "}
        <span className="font-medium">/keys → Agent Keys → Create</span>.
      </InlineStatus>
    );
  }

  // key_ready + probing + probe_success + probe_failed all show the panel
  return (
    <div className="space-y-3">
      {agentKey && (
        <AgentKeyPanel
          agentKey={agentKey}
          createdKey={createdKey}
          showOpenAiEnvSnippet={phase === "probe_success"}
        />
      )}

      {phase === "probing" && (
        <InlineStatus icon={<Spinner />}>
          Calling {createdKey.serviceName} with your Agent Key — the same
          path your AI tool will take.
        </InlineStatus>
      )}

      {phase === "probe_success" && (
        <InlineStatus
          tone="success"
          icon={<Check className="h-4 w-4 text-success" aria-hidden />}
        >
          End-to-end test succeeded
          {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}.
          Copy the Agent Key + Base URL above into your tool of choice.
        </InlineStatus>
      )}

      {phase === "probe_failed" && (
        <InlineStatus
          tone="warn"
          icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
        >
          {probeError ?? "The probe returned an unexpected response."}
          {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
          {" "}The Agent Key is still good — use it from your tool or retry.
        </InlineStatus>
      )}
    </div>
  );
}

function Spinner() {
  return (
    <Loader2
      className="h-4 w-4 animate-spin text-muted-foreground"
      aria-hidden
    />
  );
}

function InlineStatus({
  tone = "neutral",
  icon,
  children,
}: {
  readonly tone?: "neutral" | "success" | "warn";
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
  showOpenAiEnvSnippet,
}: {
  readonly agentKey: ApiKeyCreateResponse;
  readonly createdKey: CreatedKey;
  readonly showOpenAiEnvSnippet: boolean;
}) {
  const proxyUrl = proxyBaseUrl(createdKey.slug);
  const isOpenAiShaped = OPENAI_SHAPED_HINTS.test(createdKey.slug);
  return (
    <div className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="space-y-1">
        <p className="text-[12px] font-semibold text-foreground">
          Your new Agent Key
        </p>
        <p className="text-[11px] text-muted-foreground">
          Save this now. The full secret is only shown here once. Scoped to{" "}
          <span className="font-medium text-foreground">
            {createdKey.serviceName}
          </span>{" "}
          only, with the <code>proxy</code> scope.
        </p>
      </div>
      <CopyableField label="Agent Key" value={agentKey.full_key} />
      <CopyableField label="Base URL" value={proxyUrl} />
      {showOpenAiEnvSnippet && isOpenAiShaped && (
        <>
          <p className="text-[11px] text-muted-foreground pt-1">
            Most OpenAI-compatible tools (openai-python, Cursor, Continue.dev, …)
            read these two env vars.
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

type FooterAction = "onMint" | "onProbe" | "onDone";

function Footer({
  phase,
  onMint,
  onProbe,
  onDone,
}: {
  readonly phase: Phase;
  readonly onMint: () => void;
  readonly onProbe: () => void;
  readonly onDone: () => void;
}) {
  const config = footerConfig(phase);
  // Dispatch INVOKES the mapped handler. Do NOT refactor back to
  // `(a) => a === "onMint" ? onMint : …` — that returns the function
  // reference but never calls it (silent no-op click). This one bit
  // Calvin twice in a row.
  function dispatch(action: FooterAction) {
    console.info("[aha] footer click", action, "phase=", phase);
    if (action === "onMint") onMint();
    else if (action === "onProbe") onProbe();
    else onDone();
  }

  return (
    <div className="flex items-center justify-end gap-2">
      <Button
        variant="outline"
        size="lg"
        onClick={() => dispatch(config.secondary)}
        disabled={config.secondaryLabel === null || config.busy}
        className={config.secondaryLabel === null ? "invisible" : ""}
        aria-hidden={config.secondaryLabel === null}
      >
        {config.secondaryLabel ?? "placeholder"}
      </Button>
      <Button
        variant="primary"
        size="lg"
        onClick={() => dispatch(config.primary)}
        disabled={config.busy}
        isLoading={config.busy}
      >
        {config.primaryLabel}
      </Button>
    </div>
  );
}

interface FooterConfig {
  readonly secondaryLabel: string | null;
  readonly secondary: FooterAction;
  readonly primaryLabel: string;
  readonly primary: FooterAction;
  readonly busy: boolean;
}

function footerConfig(phase: Phase): FooterConfig {
  switch (phase) {
    case "connected":
      return {
        secondaryLabel: "Maybe later",
        secondary: "onDone",
        primaryLabel: "Create Agent Key",
        primary: "onMint",
        busy: false,
      };
    case "minting":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Creating…",
        primary: "onMint",
        busy: true,
      };
    case "mint_failed":
      return {
        secondaryLabel: "Maybe later",
        secondary: "onDone",
        primaryLabel: "Try again",
        primary: "onMint",
        busy: false,
      };
    case "key_ready":
      return {
        secondaryLabel: "I'll wire it myself",
        secondary: "onDone",
        primaryLabel: "Test Agent Key",
        primary: "onProbe",
        busy: false,
      };
    case "probing":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Testing…",
        primary: "onProbe",
        busy: true,
      };
    case "probe_success":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Done",
        primary: "onDone",
        busy: false,
      };
    case "probe_failed":
      return {
        secondaryLabel: "Retry test",
        secondary: "onProbe",
        primaryLabel: "Done",
        primary: "onDone",
        busy: false,
      };
  }
}

function diagnose(status: number | null, isNodeRouted: boolean): string {
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
