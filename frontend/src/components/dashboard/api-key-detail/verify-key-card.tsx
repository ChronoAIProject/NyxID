import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { useMemo, useState } from "react";
import { useKeys } from "@/hooks/use-keys";
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
import type { ApiKey, AllowedServiceInfo } from "@/types/api";
import {
  beginProxyVerification,
  recordFirstProxyCallSucceeded,
} from "@/hooks/use-proxy-onboarding";
import {
  probeAgentKey,
  proxyCallSucceeded,
  type ProbeOutcome,
} from "@/lib/proxy-probe";
import { useAuthStore } from "@/stores/auth-store";

/**
 * Sentinel slug guaranteed never to match a real user service. Used for
 * the "denied" probe — a scope-enforcing gateway should refuse the
 * bearer with a NyxID-layer 403/404 (i.e. no X-NyxID-Agent-Id on the
 * response), which is what we check for below.
 */
const DENIED_SLUG_SENTINEL = "__nyxid_test_denied__";

type ResultStatus = "idle" | "pending" | "success" | "failure";

interface ProbeResult {
  readonly slug: string;
  readonly label: string;
  readonly expected: "allowed" | "denied";
  readonly outcome: ProbeOutcome;
  readonly ok: boolean;
}

export function VerifyKeyCard({ apiKey }: { readonly apiKey: ApiKey }) {
  useUiTranslation();
  const [pastedKey, setPastedKey] = useState("");
  const [status, setStatus] = useState<ResultStatus>("idle");
  const [results, setResults] = useState<readonly ProbeResult[]>([]);
  const [overrideSlug, setOverrideSlug] = useState<string | null>(null);

  const { data: userServices } = useKeys();

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
      (apiKey.allowed_services ?? []).map((a: AllowedServiceInfo) => a.slug),
    );
    return activeUserServices
      .filter(
        (s) =>
          allowedSet.has(s.slug) ||
          (apiKey.allow_auto_connected_services &&
            s.auto_connected &&
            (apiKey.credential_source?.type === "org"
              ? s.credential_source?.type === "org" &&
                s.credential_source.org_id === apiKey.credential_source.org_id
              : s.credential_source?.type !== "org")),
      )
      .map((s) => ({ slug: s.slug, label: s.slug }));
  }, [
    apiKey.allow_all_services,
    apiKey.allowed_services,
    apiKey.allow_auto_connected_services,
    apiKey.credential_source,
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
    const auth = useAuthStore.getState();
    const userId = auth.isAuthenticated ? (auth.user?.id ?? null) : null;
    const finishVerification = beginProxyVerification(userId);
    setStatus("pending");
    setResults([]);

    async function probeSide(
      expected: "allowed" | "denied",
      slug: string,
      label: string,
    ): Promise<ProbeResult> {
      const outcome = await probeAgentKey(slug, {
        bearerToken: pastedKey.trim(),
        expectedKeyId: apiKey.id,
      });
      if (expected === "allowed" && proxyCallSucceeded(outcome)) {
        // Provider success is independent of the sentinel's verdict.
        recordFirstProxyCallSucceeded(userId);
      }
      // Keep key/scope evidence separate from provider success.
      const ok =
        expected === "allowed" ? outcome.agentKeyValid : outcome.gatewayDenied;
      return { slug, label, expected, outcome, ok };
    }

    try {
      const [allowedRes, deniedRes] = await Promise.all([
        probeSide("allowed", allowedSlug, allowedLabel),
        probeSide("denied", deniedSlug, deniedLabel),
      ]);

      const current = useAuthStore.getState();
      if (
        (current.isAuthenticated ? (current.user?.id ?? null) : null) !== userId
      )
        return;
      const nextResults = [allowedRes, deniedRes];
      setResults(nextResults);

      setStatus(allowedRes.ok && deniedRes.ok ? "success" : "failure");
    } finally {
      finishVerification();
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <ShieldCheck className="h-4 w-4 text-primary-text" />
          <CardTitle className="text-15">{translateUi("Verify key")}</CardTitle>
        </div>
        <CardDescription>
          {translateUi(
            "Check that this key reaches an allowed service and NyxID rejects a deliberately invalid slug.",
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="verify-key-paste" className="text-foreground">
            {translateUi("Agent key")}
          </Label>
          <Input
            id="verify-key-paste"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={translateUi("nyx_…")}
            value={pastedKey}
            onChange={(e) => setPastedKey(e.target.value)}
            disabled={status === "pending"}
          />
          <p className="text-13 text-muted-foreground">
            {translateUi(
              "Paste the agent key you copied when you created it. NyxID does not store this paste — it's only used for this test.",
            )}
          </p>
        </div>

        {candidateSlugs.length > 1 && (
          <div className="space-y-1.5">
            <Label htmlFor="verify-key-slug-picker" className="text-foreground">
              {translateUi("Test against")}
            </Label>
            <select
              id="verify-key-slug-picker"
              value={allowedSlug ?? ""}
              onChange={(e) => setOverrideSlug(e.target.value)}
              disabled={status === "pending"}
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-14 focus:outline-none focus:ring-2 focus:ring-ring"
            >
              {candidateSlugs.map((c) => (
                <option key={c.slug} value={c.slug}>
                  {c.label} ({c.slug})
                </option>
              ))}
            </select>
            <p className="text-13 text-muted-foreground">
              {translateUi(
                "Pick a service whose credential is fully configured. Activation requires a successful proxied 2xx; the invalid slug should return a NyxID 403/404.",
              )}
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
            {translateUi("Test key")}
          </Button>
          {status === "success" && (
            <Badge variant="success">
              <Check className="mr-1 h-3 w-3" />
              {translateUi("Gateway checks passed")}
            </Badge>
          )}
          {status === "failure" && (
            <Badge variant="destructive">
              <X className="mr-1 h-3 w-3" />
              {translateUi("Unexpected result")}
            </Badge>
          )}
        </div>

        {results.length > 0 && (
          <ul className="space-y-2">
            {results.map((r) => (
              <li
                key={`${r.expected}-${r.slug}`}
                className="space-y-1 rounded-lg border border-border/50 px-3 py-2"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="inline-flex items-center gap-2 text-14">
                    {r.ok ? (
                      <Check className="h-3.5 w-3.5 text-success" />
                    ) : (
                      <X className="h-3.5 w-3.5 text-destructive" />
                    )}
                    <span className="font-mono text-foreground">
                      {r.outcome.httpStatus ?? "—"}
                    </span>
                    <span className="text-muted-foreground">{r.slug}</span>
                  </span>
                  <span className="type-secondary-emphasis text-muted-foreground">
                    {r.expected === "allowed"
                      ? r.ok
                        ? translateUi("allowed")
                        : translateUi("Access unverified")
                      : r.ok
                        ? translateUi("denied (expected)")
                        : translateUi("Denial unverified")}
                  </span>
                </div>
                <p className="text-13 text-muted-foreground">
                  {r.expected === "denied" && !r.ok && (
                    <>
                      {translateUi(
                        "Gateway rejection was not verified. Expected a NyxID 403/404 error.",
                      )}{" "}
                    </>
                  )}
                  {r.expected === "denied" && r.ok
                    ? translateUi(
                        "NyxID rejected the invalid slug. This does not test exclusion of another existing service.",
                      )
                    : r.outcome.diagnostic}
                </p>
              </li>
            ))}
          </ul>
        )}

        {allowedSlug === null && (
          <p className="text-13 text-muted-foreground">
            {translateUi(
              "Connect a service first so this key has something allowed to call.",
            )}
          </p>
        )}

        {status === "pending" && (
          <p className="inline-flex items-center gap-1.5 text-13 text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            {translateUi("Probing…")}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
