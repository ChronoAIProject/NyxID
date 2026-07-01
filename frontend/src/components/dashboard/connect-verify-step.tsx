import { useState } from "react";
import { AlertTriangle, Check, Loader2 } from "lucide-react";
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
 * to create a key for this ai service instead of doing it automatically.
 * test comes after key is created"). Nothing fires in the background.
 */
export type Phase =
  | "connected"       // initial — dialog title says "…connected"
  | "minting"         // Create clicked, POST in flight
  | "mint_failed"     // mint errored — retry or skip
  | "key_ready"       // mint succeeded — key/URL shown, offer to test
  | "probing"         // Test clicked, bearer probe in flight
  | "probe_success"   // probe returned 2xx — the actual end-to-end aha
  | "probe_failed";   // probe returned 4xx/5xx/timeout

export interface CreatedKey {
  readonly id: string;
  /**
   * The user's user_service slug — used for the proxy URL. Repeat
   * connects of the same catalog entry get suffixed (`llm-openai-codex-2`,
   * `-3`, …) so proxy routes stay unique. That's why we also keep
   * `catalogSlug` around for icon lookup.
   */
  readonly slug: string;
  /**
   * Original catalog slug (from the selected entry, or
   * `KeyInfo.catalog_service_slug`). Drives `<ServiceIcon>` — repeat
   * connects still get the correct brand glyph. Falls back to `slug`
   * when we don't have a catalog match.
   */
  readonly catalogSlug: string;
  readonly serviceName: string;
  /**
   * How the underlying credential was established. Only used by
   * `diagnose()` today (device_code / oauth flows imply the upstream
   * already verified the user, so a 4xx from the downstream is more
   * likely a path mismatch than a bad credential).
   */
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
 * Wave-aha-1 A4+ — post-connect aha flow. Every transition is gated
 * behind an explicit user click; nothing background-fires.
 *
 * Layout is intentionally minimal per Calvin's design feedback:
 *   - DialogTitle carries the current phase (never repeats "connected"
 *     after the initial state).
 *   - DialogDescription (owned by the parent dialog) carries a one-line
 *     "what next" hint per phase.
 *   - This component renders ONLY the actionable body: an inline
 *     spinner/error line for transient phases, and the AgentKeyPanel
 *     from `key_ready` onward. No decorative banners duplicating the
 *     title.
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
          setAgentKey(key);
          setPhase("key_ready");
        },
        onError: (err) => {
          setMintError(err instanceof Error ? err.message : String(err));
          setPhase("mint_failed");
        },
      },
    );
  }

  async function triggerProbe() {
    if (!agentKey) return;
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
        // Bearer = the just-minted Agent Key. credentials:"omit" so
        // we don't accidentally fall back to the session cookie — we
        // want to prove the exact path the user's TOOL will take.
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

/**
 * Phase body. Empty for `connected` (the DialogTitle + description are
 * self-sufficient — no need for a decorative banner). Inline
 * spinner/error lines for transient phases. AgentKeyPanel from
 * `key_ready` onward is the actual output content.
 */
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
  return (
    <>
      {phase === "minting" && (
        <InlineStatus tone="neutral" icon={<Spinner />}>
          Minting a scoped key with the <code>proxy</code> scope and
          access to {createdKey.serviceName} only.
        </InlineStatus>
      )}

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

      {agentKey && (
        <>
          <AgentKeyPanel
            agentKey={agentKey}
            createdKey={createdKey}
            showOpenAiEnvSnippet={phase === "probe_success"}
          />

          {phase === "probing" && (
            <InlineStatus tone="neutral" icon={<Spinner />}>
              Calling {createdKey.serviceName} with the new Agent Key —
              the same path your AI tool will take.
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
              {probeError ??
                "The probe returned an unexpected response."}
              {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
              {" "}The Agent Key is still good — use it from your tool or
              retry the test.
            </InlineStatus>
          )}
        </>
      )}
    </>
  );
}

function Spinner() {
  return (
    <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden />
  );
}

function InlineStatus({
  tone,
  icon,
  children,
}: {
  readonly tone: "neutral" | "success" | "warn";
  readonly icon: React.ReactNode;
  readonly children: React.ReactNode;
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
  return (
    <div className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="space-y-1">
        <p className="text-[12px] font-semibold text-foreground">
          Your new Agent Key
        </p>
        <p className="text-[11px] text-muted-foreground">
          Save this now. The full secret is only shown here once — only the
          prefix is visible afterwards. Scoped to{" "}
          <span className="font-medium text-foreground">
            {createdKey.serviceName}
          </span>{" "}
          only, with the <code>proxy</code> scope.
        </p>
      </div>
      <CopyableField label="Agent Key" value={agentKey.full_key} />

      <div className="space-y-1 pt-1">
        <p className="text-[12px] font-semibold text-foreground">Proxy URL</p>
        <p className="text-[11px] text-muted-foreground">
          Point your AI tool at this URL instead of the service&apos;s direct
          API. NyxID brokers the call.
        </p>
      </div>
      <CopyableField label="Base URL" value={proxyUrl} />

      {showOpenAiEnvSnippet ? (
        <>
          <div className="space-y-1 pt-1">
            <p className="text-[12px] font-semibold text-foreground">
              Wire it up
            </p>
            <p className="text-[11px] text-muted-foreground">
              Most OpenAI-compatible tools (openai-python, Cursor,
              Continue.dev, …) read these two env vars.
            </p>
          </div>
          <CopyableField
            label="Shell env"
            value={`export OPENAI_API_KEY="${agentKey.full_key}"\nexport OPENAI_BASE_URL="${proxyUrl}/v1"`}
          />
        </>
      ) : (
        <div className="space-y-1 pt-1">
          <p className="text-[12px] font-semibold text-foreground">
            Wire it up
          </p>
          <p className="text-[11px] text-muted-foreground">
            Paste the Agent Key + Base URL above into your tool&apos;s
            provider configuration. Each tool wires this differently — check
            its docs if you&apos;re unsure.
          </p>
        </div>
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
  // Uniform 2-button footer across every phase: [outline secondary]
  // [primary]. Same size (`size="lg"`) on both so the primary never
  // shifts horizontally between transitions. Secondary slot renders
  // invisible when a phase has only one meaningful action.
  const config = footerConfig(phase);
  const dispatch = (a: FooterAction) =>
    a === "onMint" ? onMint : a === "onProbe" ? onProbe : onDone;

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
