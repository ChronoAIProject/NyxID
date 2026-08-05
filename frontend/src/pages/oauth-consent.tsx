import { useMemo, useState, type ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { AlertTriangle, ShieldCheck } from "lucide-react";
import {
  OAUTH_SCOPE_META,
  scopeRiskBadgeVariant,
  scopeRiskLabel,
} from "@/lib/constants";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useUserServices } from "@/hooks/use-user-services";
import { oauthConsentServiceAccessSchema } from "@/schemas/oauth-consent";

/**
 * Standalone shell for the consent surface, matching `login-device.tsx` — the
 * sibling "a human reviews and approves an access request" page. The
 * `bg-background` is load-bearing: the document body carries an inline dark
 * colour as an anti-flash guard for the never-themed public pages, so a themed
 * standalone page that doesn't paint its own canvas renders a light card on a
 * black void.
 */
function ConsentShell({ children }: { readonly children: ReactNode }) {
  return (
    <main
      className="flex min-h-dvh items-start justify-center bg-background px-4 py-8 text-foreground sm:items-center sm:py-10"
      style={{
        paddingTop: "max(2rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className="flex w-full max-w-xl flex-col gap-5">
        <div className="flex justify-center">
          <NyxidLogo className="h-8 w-auto" />
        </div>
        {children}
      </div>
    </main>
  );
}

/**
 * Fill for sections nested inside the Card. `bg-overlay` is the theme-aware
 * successor to the `white/[α]` idiom (app.css "Interactive chrome"): identical
 * in dark, black-alpha on light, so the nested layer survives both themes.
 */
const NESTED_SECTION = "bg-overlay";

function readParam(search: URLSearchParams, key: string): string {
  return search.get(key) ?? "";
}

function parseHost(uri: string): string {
  try {
    return new URL(uri).host;
  } catch {
    return "Unknown";
  }
}

interface ConsentServiceDisplay {
  readonly label?: string | null;
  readonly slug: string;
  readonly catalog_service_name?: string | null;
  readonly credential_source: {
    readonly type: "personal" | "org";
    readonly org_name?: string;
  };
}

/// Human-readable primary text for a service row. Never render the raw
/// user slug as the primary label (issue #1121).
function serviceDisplayName(service: ConsentServiceDisplay): string {
  return service.label || service.catalog_service_name || service.slug;
}

function serviceSecondaryText(service: ConsentServiceDisplay): string {
  const parts = [service.catalog_service_name, service.slug].filter(
    (part): part is string =>
      Boolean(part) && part !== serviceDisplayName(service),
  );
  return parts.join(" · ");
}

function serviceOrgName(service: ConsentServiceDisplay): string | null {
  return service.credential_source.type === "org"
    ? (service.credential_source.org_name ?? "Organization")
    : null;
}

export function OAuthConsentPage() {
  useApplyTheme();
  const { data: userServices, isLoading: userServicesLoading } =
    useUserServices();
  // The consent page renders once per authorize redirect; capture every
  // repeated query value in one immutable snapshot so downstream memos keep
  // stable array identities across local state updates.
  const [authorizeQuery] = useState(() => {
    const search = new URLSearchParams(window.location.search);
    return {
      search,
      resources: search.getAll("resource"),
      preselectServiceIds: search.getAll("preselect_service_ids"),
      unmatchedDefaults: search.getAll("unmatched_defaults"),
      requiredServiceIds: search.getAll("required_service_ids"),
      currentBindingServiceIds: search.getAll("current_binding_service_ids"),
    };
  });
  const {
    search,
    resources,
    preselectServiceIds,
    unmatchedDefaults,
    requiredServiceIds,
    currentBindingServiceIds,
  } = authorizeQuery;

  const responseType = readParam(search, "response_type");
  const clientId = readParam(search, "client_id");
  const clientName = readParam(search, "client_name") || clientId;
  const redirectUri = readParam(search, "redirect_uri");
  const scope = readParam(search, "scope");
  const state = search.get("state") ?? "";
  const codeChallenge = readParam(search, "code_challenge");
  const codeChallengeMethod = readParam(search, "code_challenge_method");
  const nonce = search.get("nonce") ?? "";
  const prompt = search.get("prompt") ?? "";
  const externalSubjectPlatform = search.get("external_subject_platform") ?? "";
  const externalSubjectTenant = search.get("external_subject_tenant") ?? "";
  const externalSubjectExternalUserId =
    search.get("external_subject_external_user_id") ?? "";
  const bindingGrantId = search.get("binding_grant_id") ?? "";
  const consentRequest = search.get("consent_request") ?? "";
  // Server-resolved hints: the app's declared default services matched to
  // this user (pre-selected), and declared services the user has no match
  // for (informational only).
  const bindingReview =
    search.get("binding_review") === "true" && Boolean(bindingGrantId);
  const currentBindingAllowsAllServices =
    search.get("current_binding_allow_all_services") === "true";
  const isLarkBinding = externalSubjectPlatform.toLowerCase() === "lark";
  const [allowAllServices, setAllowAllServices] = useState(
    bindingReview && currentBindingAllowsAllServices,
  );
  const [customize, setCustomize] = useState(bindingReview);
  const [selectedServiceIds, setSelectedServiceIds] = useState<
    readonly string[]
  >(() =>
    Array.from(
      new Set([
        ...preselectServiceIds,
        ...currentBindingServiceIds,
        ...requiredServiceIds,
      ]),
    ),
  );
  const [deselectedServiceIds, setDeselectedServiceIds] = useState<
    readonly string[]
  >([]);

  const missing =
    !responseType ||
    !clientId ||
    !redirectUri ||
    !scope ||
    !codeChallenge ||
    !codeChallengeMethod ||
    !consentRequest;

  const scopes = scope.split(/\s+/).filter(Boolean);
  const redirectHost = parseHost(redirectUri);
  const selectableServices = useMemo(
    () =>
      (userServices ?? [])
        .filter(
          (service) =>
            service.is_active &&
            (service.credential_source.type === "personal" ||
              service.credential_source.allowed),
        )
        .sort((a, b) =>
          serviceDisplayName(a).localeCompare(
            serviceDisplayName(b),
            undefined,
            {
              sensitivity: "base",
            },
          ),
        ),
    [userServices],
  );
  const resourceSelectedServiceIds = useMemo(() => {
    const requested = new Set(resources);
    return selectableServices
      .filter((service) => requested.has(service.resource_uri))
      .map((service) => service.id);
  }, [resources, selectableServices]);
  const effectiveSelectedServiceIds = useMemo(
    () =>
      Array.from(
        new Set([
          ...requiredServiceIds,
          ...resourceSelectedServiceIds,
          ...selectedServiceIds,
        ]),
      ).filter(
        (id) =>
          requiredServiceIds.includes(id) || !deselectedServiceIds.includes(id),
      ),
    [
      deselectedServiceIds,
      requiredServiceIds,
      resourceSelectedServiceIds,
      selectedServiceIds,
    ],
  );
  const serviceAccess = oauthConsentServiceAccessSchema.parse({
    allow_all_services: allowAllServices,
    allowed_service_ids: effectiveSelectedServiceIds,
  });
  // Rows for the read-only summary: granted services with display names,
  // marked when the app itself requested them (via declared defaults or
  // RFC 8707 resource params).
  const summaryServices = useMemo(
    () =>
      effectiveSelectedServiceIds.map((id) => {
        const service = selectableServices.find((item) => item.id === id);
        return {
          id,
          primary: service ? serviceDisplayName(service) : id,
          secondary: service ? serviceSecondaryText(service) : "",
          orgName: service ? serviceOrgName(service) : null,
          requestedByApp:
            preselectServiceIds.includes(id) ||
            resourceSelectedServiceIds.includes(id) ||
            requiredServiceIds.includes(id),
          requiredByApp:
            resourceSelectedServiceIds.includes(id) ||
            requiredServiceIds.includes(id),
          currentlyAuthorized:
            currentBindingAllowsAllServices ||
            currentBindingServiceIds.includes(id),
          newlySelected:
            bindingReview &&
            !currentBindingAllowsAllServices &&
            !currentBindingServiceIds.includes(id),
        };
      }),
    [
      effectiveSelectedServiceIds,
      preselectServiceIds,
      requiredServiceIds,
      resourceSelectedServiceIds,
      selectableServices,
      bindingReview,
      currentBindingAllowsAllServices,
      currentBindingServiceIds,
    ],
  );

  function toggleService(serviceId: string, checked: boolean) {
    if (!checked && requiredServiceIds.includes(serviceId)) return;
    setSelectedServiceIds((current) => {
      if (checked) {
        return current.includes(serviceId) ? current : [...current, serviceId];
      }
      return current.filter((id) => id !== serviceId);
    });
    setDeselectedServiceIds((current) => {
      if (checked) {
        return current.filter((id) => id !== serviceId);
      }
      return current.includes(serviceId) ? current : [...current, serviceId];
    });
  }

  if (missing) {
    return (
      <ConsentShell>
        <header className="flex flex-col gap-2 text-center">
          <h1 className="text-[22px] font-bold leading-tight tracking-tight text-foreground sm:text-[28px]">
            Invalid consent request
          </h1>
        </header>
        <ErrorBanner message="Missing required OAuth parameters. Please restart the sign-in flow." />
      </ConsentShell>
    );
  }

  return (
    <ConsentShell>
      <header className="flex flex-col gap-2 text-center">
        <h1 className="text-[22px] font-bold leading-tight tracking-tight text-foreground sm:text-[28px]">
          {bindingReview
            ? isLarkBinding
              ? "Review Lark bot access"
              : "Review application access"
            : isLarkBinding
              ? "Authorize Lark bot"
              : "Authorize application"}
        </h1>
        <p className="mx-auto max-w-md text-[12px] text-muted-foreground">
          {bindingReview ? (
            <>
              Review the NyxID services available to{" "}
              <span className="font-medium text-foreground">{clientName}</span>.
            </>
          ) : (
            <>
              <span className="font-medium text-foreground">{clientName}</span>{" "}
              wants to access your account via OAuth.
            </>
          )}
        </p>
      </header>

      <Card className="border-border/50">
        <CardContent className="flex flex-col gap-4 pt-4">
          <DetailSection title="App details" className={NESTED_SECTION}>
            <DetailRow label="Application" value={clientName} />
            <DetailRow label="Redirect host" value={redirectHost} />
            <DetailRow label="Client ID" value={clientId} mono copyable />
            <DetailRow label="Redirect URI" value={redirectUri} mono copyable />
          </DetailSection>

          <DetailSection title="Requested access" className={NESTED_SECTION}>
            {scopes.map((item) => {
                const meta = OAUTH_SCOPE_META[item] ?? {
                  title: "Custom permission",
                  description:
                    "This app is requesting a non-standard permission.",
                  risk: "medium" as const,
                };
                return (
                  <div key={`meta-${item}`} className="px-4 py-2.5">
                    <div className="flex items-start justify-between gap-3">
                      <p className="min-w-0 break-words text-[12px] font-medium text-foreground">
                        {meta.title}
                      </p>
                      <Badge
                        variant={scopeRiskBadgeVariant(meta.risk)}
                        className="shrink-0"
                      >
                        {scopeRiskLabel(meta.risk)}
                      </Badge>
                    </div>
                    <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
                      {meta.description}
                    </p>
                    <p className="mt-1.5 break-all font-mono text-[11px] text-text-tertiary">
                      {item}
                    </p>
                  </div>
                );
              })}
          </DetailSection>

          <DetailSection
            title="Service access"
            className={NESTED_SECTION}
            action={
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="shrink-0"
                onClick={() => setCustomize((current) => !current)}
              >
                {customize ? "Done" : "Customize"}
              </Button>
            }
          >
            <p className="px-4 py-2.5 text-[12px] leading-relaxed text-muted-foreground">
              {serviceAccess.allow_all_services
                ? bindingReview && currentBindingAllowsAllServices
                  ? "This binding currently authorizes all available services."
                  : "This app will be able to use all of your available services through the proxy."
                : summaryServices.length > 0
                  ? bindingReview
                    ? "Review the current grant and select any additional services."
                    : "This app will be able to use these services through the proxy:"
                  : "No service access requested. This app only signs you in."}
            </p>

            {!customize &&
              !serviceAccess.allow_all_services &&
              (summaryServices.length > 0 || unmatchedDefaults.length > 0) && (
                <div className="divide-y divide-border/30">
                  {summaryServices.map((item) => (
                    <div
                      key={item.id}
                      className="flex items-start justify-between gap-3 px-4 py-2.5"
                    >
                      <div className="min-w-0">
                        <p className="break-words text-[12px] font-medium text-foreground">
                          {item.primary}
                        </p>
                        {item.secondary && (
                          <p className="mt-0.5 break-words text-[11px] text-muted-foreground">
                            {item.secondary}
                          </p>
                        )}
                        {item.orgName && (
                          <div className="mt-1 flex flex-wrap items-center gap-1.5">
                            <Badge variant="secondary" className="text-[10px]">
                              Org
                            </Badge>
                            <span className="break-words text-[11px] text-muted-foreground">
                              {item.orgName}
                            </span>
                          </div>
                        )}
                      </div>
                      <div className="flex shrink-0 flex-wrap justify-end gap-1">
                        {item.currentlyAuthorized && bindingReview && (
                          <Badge variant="secondary" className="text-[10px]">
                            Authorized now
                          </Badge>
                        )}
                        {item.requiredByApp ? (
                          <Badge variant="secondary" className="text-[10px]">
                            Required by app
                          </Badge>
                        ) : (
                          item.requestedByApp && (
                            <Badge variant="secondary" className="text-[10px]">
                              Requested by app
                            </Badge>
                          )
                        )}
                        {item.newlySelected && (
                          <Badge variant="accent" className="text-[10px]">
                            New
                          </Badge>
                        )}
                      </div>
                    </div>
                  ))}
                  {unmatchedDefaults.map((name) => (
                    <div key={`unmatched-${name}`} className="px-4 py-2.5">
                      <p className="break-words text-[12px] leading-relaxed text-muted-foreground">
                        <span className="font-medium text-foreground">
                          {name}
                        </span>{" "}
                        — requested by this app, but you have no matching
                        service in your account.
                      </p>
                    </div>
                  ))}
                </div>
              )}

            {customize && (
              <div className="divide-y divide-border/30">
                <div className="flex items-center justify-between gap-2 px-4 py-2.5">
                  <Label htmlFor="oauth-allow-all-services">All services</Label>
                  <Switch
                    id="oauth-allow-all-services"
                    aria-label="All services"
                    checked={allowAllServices}
                    onCheckedChange={setAllowAllServices}
                  />
                </div>

                {!allowAllServices && (
                  <div className="divide-y divide-border/30">
                    {userServicesLoading ? (
                      <p className="px-4 py-2.5 text-[12px] text-muted-foreground">
                        Loading services...
                      </p>
                    ) : selectableServices.length > 0 ? (
                      selectableServices.map((service) => {
                        const orgName = serviceOrgName(service);
                        return (
                          <div
                            key={service.id}
                            className="flex items-start gap-2.5 px-4 py-2.5"
                          >
                            <Checkbox
                              id={`oauth-service-${service.id}`}
                              checked={effectiveSelectedServiceIds.includes(
                                service.id,
                              )}
                              disabled={requiredServiceIds.includes(service.id)}
                              onCheckedChange={(checked) =>
                                toggleService(service.id, checked === true)
                              }
                            />
                            <div className="min-w-0">
                              <Label
                                htmlFor={`oauth-service-${service.id}`}
                                className="cursor-pointer text-[12px] leading-5 text-foreground"
                              >
                                <span className="block break-words font-medium">
                                  {serviceDisplayName(service)}
                                </span>
                                {serviceSecondaryText(service) && (
                                  <span className="block break-words text-[11px] font-normal text-muted-foreground">
                                    {serviceSecondaryText(service)}
                                  </span>
                                )}
                              </Label>
                              {orgName && (
                                <div className="mt-1 flex flex-wrap items-center gap-1.5">
                                  <Badge
                                    variant="secondary"
                                    className="text-[10px]"
                                  >
                                    Org
                                  </Badge>
                                  <span className="break-words text-[11px] font-normal text-muted-foreground">
                                    {orgName}
                                  </span>
                                </div>
                              )}
                              <div className="mt-1 flex flex-wrap gap-1">
                                {bindingReview &&
                                  (currentBindingAllowsAllServices ||
                                    currentBindingServiceIds.includes(
                                      service.id,
                                    )) && (
                                    <Badge
                                      variant="secondary"
                                      className="text-[10px]"
                                    >
                                      Authorized now
                                    </Badge>
                                  )}
                                {requiredServiceIds.includes(service.id) && (
                                  <Badge
                                    variant="secondary"
                                    className="text-[10px]"
                                  >
                                    Required by app
                                  </Badge>
                                )}
                                {bindingReview &&
                                  effectiveSelectedServiceIds.includes(
                                    service.id,
                                  ) &&
                                  !currentBindingAllowsAllServices &&
                                  !currentBindingServiceIds.includes(
                                    service.id,
                                  ) &&
                                  !requiredServiceIds.includes(service.id) && (
                                    <Badge
                                      variant="accent"
                                      className="text-[10px]"
                                    >
                                      New
                                    </Badge>
                                  )}
                              </div>
                            </div>
                          </div>
                        );
                      })
                    ) : (
                      <p className="px-4 py-2.5 text-[12px] text-muted-foreground">
                        No active services are available.
                      </p>
                    )}
                  </div>
                )}
              </div>
            )}
          </DetailSection>

          <div className="flex gap-3 rounded-xl border border-warning/15 bg-warning/[0.04] px-4 py-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-warning/10">
              <AlertTriangle className="h-4 w-4 text-warning" />
            </div>
            <p className="text-[12px] leading-relaxed text-warning">
              Only continue if you trust this application.
            </p>
          </div>

          <form
            method="POST"
            action="/oauth/authorize/decision"
            className="flex flex-col gap-2 sm:flex-row sm:justify-end"
          >
            <input type="hidden" name="response_type" value={responseType} />
            <input type="hidden" name="client_id" value={clientId} />
            <input type="hidden" name="redirect_uri" value={redirectUri} />
            <input type="hidden" name="scope" value={scope} />
            <input type="hidden" name="state" value={state} />
            <input type="hidden" name="code_challenge" value={codeChallenge} />
            <input
              type="hidden"
              name="code_challenge_method"
              value={codeChallengeMethod}
            />
            <input type="hidden" name="nonce" value={nonce} />
            <input
              type="hidden"
              name="consent_request"
              value={consentRequest}
            />
            {prompt && <input type="hidden" name="prompt" value={prompt} />}
            {externalSubjectPlatform && (
              <input
                type="hidden"
                name="external_subject_platform"
                value={externalSubjectPlatform}
              />
            )}
            {externalSubjectTenant && (
              <input
                type="hidden"
                name="external_subject_tenant"
                value={externalSubjectTenant}
              />
            )}
            {externalSubjectExternalUserId && (
              <input
                type="hidden"
                name="external_subject_external_user_id"
                value={externalSubjectExternalUserId}
              />
            )}
            {bindingGrantId && (
              <input
                type="hidden"
                name="binding_grant_id"
                value={bindingGrantId}
              />
            )}
            <input
              type="hidden"
              name="allow_all_services"
              value={serviceAccess.allow_all_services ? "true" : "false"}
            />
            {!serviceAccess.allow_all_services &&
              serviceAccess.allowed_service_ids.map((serviceId) => (
                <input
                  key={serviceId}
                  type="hidden"
                  name="allowed_service_ids"
                  value={serviceId}
                />
              ))}
            {resources.map((resource) => (
              <input
                key={resource}
                type="hidden"
                name="resource"
                value={resource}
              />
            ))}

            <Button
              type="submit"
              variant="outline"
              name="decision"
              value="deny"
              className="w-full sm:w-auto"
            >
              {bindingReview ? "Cancel" : "Deny"}
            </Button>
            <Button
              variant="primary"
              type="submit"
              name="decision"
              value="allow"
              className="w-full sm:w-auto"
            >
              <ButtonIcon variant="primary">
                <ShieldCheck />
              </ButtonIcon>
              {bindingReview ? "Update access" : "Allow"}
            </Button>
          </form>
        </CardContent>
      </Card>

      <p className="text-center text-[11px] text-text-tertiary">
        You can revoke this access at any time from Authorized Applications.
      </p>
    </ConsentShell>
  );
}
