import type { ReactNode } from "react";
import { useState } from "react";
import { Check, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
import { ServiceIcon } from "@/components/service-icons";
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
 * (per Calvin's feedback: "the lift should be to ask them if they want
 * to create a key for this ai service instead of doing it automatically.
 * test comes after key is created"). No background activity ever runs
 * without the user pulling a trigger first.
 */
type Phase =
  | "connected"       // initial — service is connected, ask if they want an Agent Key
  | "minting"         // user clicked Create Agent Key, mint in flight
  | "mint_failed"     // mint errored — show retry + skip
  | "key_ready"       // mint succeeded — show key + offer to test
  | "probing"         // user clicked Test — probe in flight
  | "probe_success"   // probe returned 2xx — the actual end-to-end aha
  | "probe_failed";   // probe returned 4xx/5xx/timeout — show diagnose hint + retry

export interface CreatedKey {
  readonly id: string;
  /**
   * The user's user_service slug — used for the proxy URL. Note this
   * may differ from `catalogSlug`: users who connect the same catalog
   * service twice get suffixed slugs (`llm-openai-codex-2`,
   * `llm-openai-codex-3`, etc.) so proxy routes stay unique.
   */
  readonly slug: string;
  /**
   * The original catalog slug (from `catalog_service_slug` on KeyInfo,
   * or the selected catalog entry). Used for ServiceIcon lookup so the
   * brand glyph resolves correctly even for `-2`/`-3` repeat connects.
   * Falls back to `slug` when we don't have a catalog match.
   */
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
 * Even for services where we know the openai-shaped probe path won't
 * return 200 (Codex's chat backend, custom endpoints), the test is
 * still useful — a 4xx from the DOWNSTREAM proves NyxID's bearer-auth
 * middleware accepted the Agent Key and forwarded the request. That's
 * the half of the path the user actually cares about (their tool talks
 * to NyxID, NyxID talks to the downstream). Diagnose copy explains
 * outcomes honestly per HTTP status. Calvin: "the modal needs to be
 * able to test the agent key" — so we always offer it.
 */
function probePath(createdKey: CreatedKey): string {
  const openAiProbe = probePathForSlug(createdKey.slug);
  // Fall back to root path when the slug isn't openai-shaped. Some
  // downstreams will 404, some will return a landing page, some will
  // 401 — all of them prove the Agent Key was accepted by NyxID.
  return openAiProbe || "";
}

/**
 * Wave-aha-1 A4+ (opt-in rework) — post-connect aha moment, but every
 * step requires the user to pull a trigger:
 *
 *   1. Service is connected → ask "Create an Agent Key for this?" with
 *      [Create Agent Key] [Maybe later]. No background mint.
 *   2. User clicks → mint runs visibly. If it fails, show the error
 *      and offer retry — instead of leaving a spinner spinning forever.
 *   3. Mint succeeds → show full key + proxy URL + (where applicable)
 *      env-var snippet, with [Test connection] / [I'll wire it myself].
 *   4. User clicks Test → bearer probe runs visibly. Success or precise
 *      failure copy. Retry available either way.
 *
 * The earlier auto-mint version hung silently when the mint request
 * never reached the BE (no surfaced error, no user agency). This
 * version makes failures impossible to miss because they always follow
 * a deliberate click.
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

    const url = `${proxyBaseUrl(createdKey.slug)}/${probePath(createdKey)}`;
    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort(),
      PROBE_TIMEOUT_MS,
    );

    let status: number | null = null;
    try {
      const res = await fetch(url, {
        method: "GET",
        // Bearer = the user's just-minted Agent Key. credentials:"omit"
        // so we don't accidentally fall back to the session cookie —
        // we want to prove the bearer path the user's TOOL will take.
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
      {/* Top status banner — what just happened / what we're doing now */}
      <StatusBanner
        phase={phase}
        createdKey={createdKey}
        mintError={mintError}
        probeError={probeError}
        httpStatus={httpStatus}
      />

      {/* Once minted, the Agent Key + URL panel stays visible from
          key_ready through probe_success / probe_failed so the user can
          copy it regardless of test outcome. */}
      {agentKey && (
        <AgentKeyPanel
          agentKey={agentKey}
          createdKey={createdKey}
          showOpenAiEnvSnippet={phase === "probe_success"}
        />
      )}

      {/* Action buttons — gate every transition behind an explicit click. */}
      <Footer
        phase={phase}
        onMint={triggerMint}
        onProbe={triggerProbe}
        onDone={onDone}
      />
    </div>
  );
}

function StatusBanner({
  phase,
  createdKey,
  mintError,
  probeError,
  httpStatus,
}: {
  readonly phase: Phase;
  readonly createdKey: CreatedKey;
  readonly mintError: string | null;
  readonly probeError: string | null;
  readonly httpStatus: number | null;
}) {
  const serviceName = createdKey.serviceName;
  // Quoted + icon-prefixed service name — matches the DialogTitle styling
  // so it's obvious that "OpenAI Codex API" is the THING that got
  // connected, not part of the verb.
  const quotedServiceTitle: ReactNode = (
    <span className="inline-flex items-center gap-1.5">
      <ServiceIcon
        slug={createdKey.catalogSlug}
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
      />
      <span>
        <span className="text-muted-foreground">&ldquo;</span>
        {serviceName}
        <span className="text-muted-foreground">&rdquo;</span>
        {" "}connected
      </span>
    </span>
  );

  switch (phase) {
    case "connected":
      return (
        <Banner icon="check" tone="success" title={quotedServiceTitle}>
          NyxID has stored your credentials. To let your AI tools use this
          connection, you&apos;ll need an Agent Key scoped to this service.
        </Banner>
      );
    case "minting":
      return (
        <Banner icon="spinner" tone="neutral" title="Creating your Agent Key…">
          Minting a scoped key with the <code>proxy</code> scope and access
          to {serviceName} only.
        </Banner>
      );
    case "mint_failed":
      return (
        <Banner
          icon="warn"
          tone="warn"
          title="Couldn’t create the Agent Key"
        >
          {mintError ??
            "The server didn't accept the request."}{" "}
          You can retry, or skip and create one later from{" "}
          <span className="font-medium">/keys → Agent Keys → Create</span>.
        </Banner>
      );
    case "key_ready":
      return (
        <Banner
          icon="check"
          tone="success"
          title={`Agent Key created for ${serviceName}`}
        >
          Save the key now (it&apos;s only shown once). To prove the
          end-to-end path works, run a test below — or skip and wire it
          straight into your AI tool.
        </Banner>
      );
    case "probing":
      return (
        <Banner
          icon="spinner"
          tone="neutral"
          title="Testing the end-to-end path…"
        >
          Calling {serviceName} with the new Agent Key as bearer — the same
          path your AI tool will take. This takes a couple of seconds.
        </Banner>
      );
    case "probe_success":
      return (
        <Banner
          icon="check"
          tone="success"
          title={`Your AI tool can now call ${serviceName} through NyxID`}
        >
          End-to-end test succeeded
          {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}.
          Copy the Agent Key + Base URL into your tool of choice — you&apos;re done.
        </Banner>
      );
    case "probe_failed":
      return (
        <Banner
          icon="warn"
          tone="warn"
          title="End-to-end test didn’t succeed"
        >
          <span>
            {probeError ??
              "The probe returned an unexpected response."}
            {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
            {" "}The Agent Key is still good — copy it above and use it from
            your tool, or retry the test below.
          </span>
        </Banner>
      );
  }
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
  // [primary]. Same size (`size="lg"`) on both so nothing jumps between
  // transitions. Labels + actions change per phase; layout is constant.
  // Secondary slot stays rendered but invisible when there's no
  // meaningful alternative — keeps the primary button in the same
  // horizontal position from phase to phase.
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

/**
 * One place to see every label + action per phase. Data-driven layout
 * keeps the JSX compact and prevents subtle drift between phases.
 */
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
        // Secondary retries the probe; primary dismisses. Users who
        // don't want to retry can Done and troubleshoot from /keys.
        secondaryLabel: "Retry test",
        secondary: "onProbe",
        primaryLabel: "Done",
        primary: "onDone",
        busy: false,
      };
  }
}

function Banner({
  icon,
  tone,
  title,
  children,
}: {
  readonly icon: "spinner" | "check" | "warn";
  readonly tone: "neutral" | "success" | "warn";
  readonly title: ReactNode;
  readonly children: ReactNode;
}) {
  const tonalRing =
    tone === "success"
      ? "bg-success/20"
      : tone === "warn"
        ? "bg-warning/20"
        : "bg-muted";
  const glyph =
    icon === "spinner" ? (
      <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden />
    ) : icon === "check" ? (
      <Check className="h-4 w-4 text-success" aria-hidden />
    ) : (
      <X className="h-4 w-4 text-warning" aria-hidden />
    );
  return (
    <div className="rounded-xl border border-border/50 bg-card p-4">
      <div className="flex items-start gap-3">
        <span
          className={`mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full ${tonalRing}`}
        >
          {glyph}
        </span>
        <div className="space-y-1">
          <p className="text-[13px] font-semibold text-foreground">{title}</p>
          <p className="text-[12px] text-muted-foreground">{children}</p>
        </div>
      </div>
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
    return `The downstream returned a server error (HTTP ${String(status)}). NyxID's part of the path is fine; the upstream service is having issues right now.`;
  }
  return `The probe returned an unexpected HTTP ${String(status)}.`;
}
