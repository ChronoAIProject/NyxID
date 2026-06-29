import { useEffect, useMemo, useRef, useState } from "react";
import { useUserServices } from "@/hooks/use-user-services";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Check, Loader2, ShieldCheck, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ApiKey, AllowedServiceInfo } from "@/types/api";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";

/**
 * Sentinel slug guaranteed never to match a real user service, so that
 * a scoped key with `allow_all_services=true` still sees a 4xx on the
 * denied-slug probe (the proxy returns 404 for an unknown slug, which
 * we accept as "scope enforced" for the denial case).
 */
const DENIED_SLUG_SENTINEL = "__nyxid_test_denied__";

/**
 * Slugs that almost always expose `/v1/models` on AI providers. Used to
 * decide whether the probe path should be `v1/models` (OpenAI-shaped API)
 * or `` (treat the downstream's root path). Mismatched services still
 * produce a non-Okay status — what matters is that a 403 vs. 200 split
 * is observable.
 */
const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const PROBE_TIMEOUT_MS = 8000;

type ResultStatus = "idle" | "pending" | "success" | "failure";

interface ProbeResult {
  readonly slug: string;
  readonly label: string;
  readonly expected: "allowed" | "denied";
  readonly status: number | null;
  readonly ok: boolean;
}

function probePathForSlug(slug: string): string {
  return OPENAI_SHAPED_HINTS.test(slug) ? "v1/models" : "";
}

function isAllowedOutcome(s: number | null): boolean {
  return s !== null && s >= 200 && s < 400;
}

function isDeniedOutcome(s: number | null): boolean {
  // 4xx (incl. 403) and 5xx both count as "the scope held": either the
  // gateway rejected the call outright (403) or the downstream returned a
  // client error because the brokered path is unknown. We never treat a
  // 2xx/3xx as a denial.
  return s !== null && (s >= 400 || s >= 500);
}

export function VerifyKeyCard({
  apiKey,
}: {
  readonly apiKey: ApiKey;
}) {
  const [pastedKey, setPastedKey] = useState("");
  const [status, setStatus] = useState<ResultStatus>("idle");
  const [results, setResults] = useState<
    readonly ProbeResult[]
  >([]);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [overrideSlug, setOverrideSlug] = useState<string | null>(null);
  const firedSuccessRef = useRef(false);
  const loadingStartDispatchedRef = useRef(false);

  const { data: userServices } = useUserServices();

  useEffect(() => {
    return () => {
      if (loadingStartDispatchedRef.current) {
        window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
        loadingStartDispatchedRef.current = false;
      }
    };
  }, []);

  // Build the candidate list of slugs the user can probe against. For a key
  // with blanket access, all the user's connected services are candidates;
  // otherwise only those listed in the key's scope. The dropdown defaults
  // to the first candidate but the user can choose any.
  const candidateSlugs = useMemo<
    readonly { slug: string; label: string }[]
  >(() => {
    const activeUserServices = (userServices ?? []).filter(
      (s) => s.is_active !== false,
    );
    if (apiKey.allow_all_services) {
      return activeUserServices.map((s) => ({
        slug: s.slug,
        label: s.slug,
      }));
    }
    const allowedSet = new Set(
      (apiKey.allowed_services ?? []).map(
        (a: AllowedServiceInfo) => a.slug,
      ),
    );
    return activeUserServices
      .filter((s) => allowedSet.has(s.slug))
      .map((s) => ({ slug: s.slug, label: s.slug }));
  }, [
    apiKey.allow_all_services,
    apiKey.allowed_services,
    userServices,
  ]);

  const allowedSlug: string | null = useMemo(() => {
    if (overrideSlug && candidateSlugs.some((c) => c.slug === overrideSlug)) {
      return overrideSlug;
    }
    return candidateSlugs[0]?.slug ?? null;
  }, [overrideSlug, candidateSlugs]);

  const allowedLabel = useMemo<string>(() => {
    const match = candidateSlugs.find((c) => c.slug === allowedSlug);
    return match?.label ?? allowedSlug ?? "(no service)";
  }, [candidateSlugs, allowedSlug]);

  const deniedSlug = DENIED_SLUG_SENTINEL;
  const deniedLabel = "denied sentinel";

  const canRun =
    pastedKey.trim().length > 0 && allowedSlug !== null && status !== "pending";

  async function runTest() {
    if (!allowedSlug || !pastedKey.trim()) return;
    setStatus("pending");
    setResults([]);
    setErrorMessage(null);
    if (firedSuccessRef.current === false) {
      window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));
      loadingStartDispatchedRef.current = true;
    }

    const allowedUrl = `/api/v1/proxy/s/${encodeURIComponent(allowedSlug)}/${probePathForSlug(allowedSlug)}`;
    const deniedUrl = `/api/v1/proxy/s/${encodeURIComponent(deniedSlug)}/`;

    const headers = (): HeadersInit => ({
      Authorization: `Bearer ${pastedKey.trim()}`,
      // Some downstreams require a content type; harmless if unused.
      "Content-Type": "application/json",
    });

    const fetchOpts = (): RequestInit & { signal?: AbortSignal } => ({
      method: "GET",
      headers: headers(),
      // We do NOT want to follow the SPA's auth context — the test must
      // use only the pasted key, so we omit credentials.
      credentials: "omit",
    });

    async function probe(
      url: string,
      expected: "allowed" | "denied",
      slug: string,
      label: string,
    ): Promise<ProbeResult> {
      const controller = new AbortController();
      const timer = setTimeout(
        () => controller.abort(),
        PROBE_TIMEOUT_MS,
      );
      let status: number | null = null;
      try {
        const res = await fetch(url, {
          ...fetchOpts(),
          signal: controller.signal,
        });
        status = res.status;
      } catch {
        status = null;
      } finally {
        clearTimeout(timer);
      }
      const ok =
        expected === "allowed"
          ? isAllowedOutcome(status)
          : isDeniedOutcome(status);
      return { slug, label, expected, status, ok };
    }

    const [allowedRes, deniedRes] = await Promise.all([
      probe(allowedUrl, "allowed", allowedSlug, allowedLabel),
      probe(deniedUrl, "denied", deniedSlug, deniedLabel),
    ]);

    const nextResults = [allowedRes, deniedRes];
    setResults(nextResults);

    const bothOk = allowedRes.ok && deniedRes.ok;
    if (bothOk) {
      setStatus("success");
      if (!firedSuccessRef.current) {
        firedSuccessRef.current = true;
        window.dispatchEvent(
          new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT),
        );
      }
    } else {
      setStatus("failure");
      const failing = nextResults.filter((r) => !r.ok);
      const explanations = failing.map((r) => {
        if (r.expected === "allowed") {
          if (r.status === 401 || r.status === 403) {
            return `Allowed service "${r.slug}" returned ${r.status}. This usually means either (a) the agent key isn't actually scoped to this slug, or (b) the service is connected but its downstream credentials aren't configured yet. Open the service from /keys to check, or pick a different service above.`;
          }
          if (r.status === 404) {
            return `Allowed service "${r.slug}" returned 404 — the slug isn't registered as a user-service for this account.`;
          }
          if (r.status === null) {
            return `Allowed probe to "${r.slug}" timed out or was blocked by the browser.`;
          }
          return `Allowed service "${r.slug}" returned ${r.status} (expected 2xx).`;
        }
        return `Denied probe returned ${r.status ?? "no response"} (expected 4xx). The gateway may be misconfigured — flag this.`;
      });
      setErrorMessage(explanations.join(" "));
    }

    if (loadingStartDispatchedRef.current) {
      window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
      loadingStartDispatchedRef.current = false;
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <ShieldCheck className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Verify key</CardTitle>
        </div>
        <CardDescription>
          Prove this key is scoped correctly — it should reach an allowed
          service and be blocked from a denied one.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="space-y-1.5">
          <label
            htmlFor="verify-key-paste"
            className="text-[11px] font-medium text-foreground"
          >
            Agent key
          </label>
          <Input
            id="verify-key-paste"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder="nyx_…"
            value={pastedKey}
            onChange={(e) => setPastedKey(e.target.value)}
            disabled={status === "pending"}
          />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Paste the agent key you copied when you created it. NyxID does not
            store this paste — it&apos;s only used for this test.
          </p>
        </div>

        {candidateSlugs.length > 1 && (
          <div className="space-y-1.5">
            <label
              htmlFor="verify-key-slug-picker"
              className="text-[11px] font-medium text-foreground"
            >
              Test against
            </label>
            <select
              id="verify-key-slug-picker"
              value={allowedSlug ?? ""}
              onChange={(e) => setOverrideSlug(e.target.value)}
              disabled={status === "pending"}
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-[12px] focus:outline-none focus:ring-2 focus:ring-ring"
            >
              {candidateSlugs.map((c) => (
                <option key={c.slug} value={c.slug}>
                  {c.label} ({c.slug})
                </option>
              ))}
            </select>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Pick a service whose downstream credential is fully configured.
              The test will probe this slug (should return 2xx) and a
              deliberately-invalid slug (should return 4xx).
            </p>
          </div>
        )}

        <div className="flex items-center gap-2">
          <Button
            variant="primary"
            onClick={runTest}
            disabled={!canRun}
            isLoading={status === "pending"}
          >
            Test key
          </Button>
          {status === "success" && (
            <Badge variant="success">
              <Check className="mr-1 h-3 w-3" />
              Scope verified
            </Badge>
          )}
          {status === "failure" && (
            <Badge variant="destructive">
              <X className="mr-1 h-3 w-3" />
              Unexpected result
            </Badge>
          )}
        </div>

        {results.length > 0 && (
          <ul className="space-y-2">
            {results.map((r) => (
              <li
                key={`${r.expected}-${r.slug}`}
                className="flex items-center justify-between rounded-lg border border-border/50 px-3 py-2"
              >
                <span className="inline-flex items-center gap-2 text-[12px]">
                  {r.ok ? (
                    <Check className="h-3.5 w-3.5 text-success" />
                  ) : (
                    <X className="h-3.5 w-3.5 text-destructive" />
                  )}
                  <span className="font-mono text-foreground">
                    {r.status ?? "—"}
                  </span>
                  <span className="text-muted-foreground">{r.slug}</span>
                </span>
                <span
                  className={cn(
                    "text-[11px] font-medium",
                    r.expected === "allowed"
                      ? "text-success/80"
                      : "text-destructive/80",
                  )}
                >
                  {r.expected === "allowed" ? "allowed" : "denied (expected)"}
                </span>
              </li>
            ))}
          </ul>
        )}

        {errorMessage && (
          <p className="text-[11px] leading-relaxed text-destructive">
            {errorMessage}
          </p>
        )}

        {allowedSlug === null && (
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Connect a service first so this key has something allowed to call.
          </p>
        )}

        {status === "pending" && (
          <p className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            Probing…
          </p>
        )}
      </CardContent>
    </Card>
  );
}
