import { useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { AlertTriangle, ShieldCheck } from "lucide-react";
import {
  OAUTH_SCOPE_META,
  scopeRiskClass,
  scopeRiskLabel,
} from "@/lib/constants";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { useUserServices } from "@/hooks/use-user-services";
import { oauthConsentServiceAccessSchema } from "@/schemas/oauth-consent";

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
  // The consent page renders once per authorize redirect; the query string
  // never changes within a mount, so URL-derived values are memoized to keep
  // referential stability for the memos below.
  const search = useMemo(() => new URLSearchParams(window.location.search), []);

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
  const consentRequest = search.get("consent_request") ?? "";
  const resources = useMemo(() => search.getAll("resource"), [search]);
  // Server-resolved hints: the app's declared default services matched to
  // this user (pre-selected), and declared services the user has no match
  // for (informational only).
  const preselectServiceIds = useMemo(
    () => search.getAll("preselect_service_ids"),
    [search],
  );
  const unmatchedDefaults = useMemo(
    () => search.getAll("unmatched_defaults"),
    [search],
  );
  const [allowAllServices, setAllowAllServices] = useState(false);
  const [customize, setCustomize] = useState(false);
  const [selectedServiceIds, setSelectedServiceIds] =
    useState<readonly string[]>(preselectServiceIds);
  const [deselectedServiceIds, setDeselectedServiceIds] = useState<
    readonly string[]
  >([]);
  const [selectedResources, setSelectedResources] = useState(resources);

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
    const requested = new Set(selectedResources);
    return selectableServices
      .filter((service) => requested.has(service.resource_uri))
      .map((service) => service.id);
  }, [selectableServices, selectedResources]);
  const effectiveSelectedServiceIds = useMemo(
    () =>
      Array.from(
        new Set([...resourceSelectedServiceIds, ...selectedServiceIds]),
      ).filter((id) => !deselectedServiceIds.includes(id)),
    [deselectedServiceIds, resourceSelectedServiceIds, selectedServiceIds],
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
            resourceSelectedServiceIds.includes(id),
        };
      }),
    [
      effectiveSelectedServiceIds,
      preselectServiceIds,
      resourceSelectedServiceIds,
      selectableServices,
    ],
  );

  function toggleService(serviceId: string, checked: boolean) {
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

  function toggleResource(resource: string) {
    setSelectedResources((current) =>
      current.includes(resource)
        ? current.filter((item) => item !== resource)
        : [...current, resource],
    );
  }

  if (missing) {
    return (
      <div
        className="mx-auto flex min-h-dvh w-full max-w-2xl items-center px-6 py-10"
        style={{
          paddingTop: "max(2.5rem, var(--sat))",
          paddingBottom: "max(2.5rem, var(--sab))",
        }}
      >
        <Card className="w-full">
          <CardHeader>
            <CardTitle>Invalid consent request</CardTitle>
            <CardDescription>
              Missing required OAuth parameters. Please restart the sign-in
              flow.
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    );
  }

  return (
    <div
      className="mx-auto flex min-h-dvh w-full max-w-2xl items-center px-6 py-10"
      style={{
        paddingTop: "max(2.5rem, var(--sat))",
        paddingBottom: "max(2.5rem, var(--sab))",
      }}
    >
      <Card className="w-full">
        <CardHeader className="space-y-4">
          <div className="flex items-center">
            <NyxidLogo className="h-7 w-auto" />
          </div>
          <CardTitle>Authorize Application</CardTitle>
          <CardDescription>
            <span className="font-medium text-foreground">{clientName}</span>{" "}
            wants to access your account via OAuth.
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-6">
          <div className="rounded-lg border border-border bg-muted px-4 py-3">
            <div className="flex items-center gap-2 text-[12px] font-medium text-foreground">
              <ShieldCheck className="h-4 w-4 text-primary" />
              App verification details
            </div>
            <div className="mt-2 space-y-1 text-xs text-muted-foreground">
              <p>
                Application:{" "}
                <span className="font-medium text-foreground">
                  {clientName}
                </span>
              </p>
              <p>
                Redirect host:{" "}
                <span className="text-foreground">{redirectHost}</span>
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Requested scopes
            </p>
            <div className="flex flex-wrap gap-2">
              {scopes.map((item) => (
                <Badge key={item} variant="secondary">
                  {item}
                </Badge>
              ))}
            </div>
          </div>

          {resources.length > 0 && (
            <div className="space-y-2">
              <p className="text-xs uppercase tracking-wide text-text-tertiary">
                Requested resources
              </p>
              <div className="space-y-2">
                {resources.map((resource) => (
                  <label
                    key={resource}
                    className="flex items-start gap-3 rounded-lg border border-border bg-muted/50 px-3 py-2"
                  >
                    <Checkbox
                      className="mt-0.5"
                      aria-label={resource}
                      checked={selectedResources.includes(resource)}
                      onCheckedChange={() => toggleResource(resource)}
                    />
                    <p className="break-all text-xs text-foreground">
                      {resource}
                    </p>
                  </label>
                ))}
              </div>
            </div>
          )}

          <div className="space-y-2">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Scope impact
            </p>
            <div className="space-y-2">
              {scopes.map((item) => {
                const meta = OAUTH_SCOPE_META[item] ?? {
                  title: "Custom permission",
                  description:
                    "This app is requesting a non-standard permission.",
                  risk: "medium" as const,
                };
                return (
                  <div
                    key={`meta-${item}`}
                    className="rounded-lg border border-border bg-muted/50 px-3 py-2"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <p className="min-w-0 break-words text-xs text-foreground pt-0.5">
                        {item}
                      </p>
                      <span
                        className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-medium ${scopeRiskClass(meta.risk)}`}
                      >
                        {scopeRiskLabel(meta.risk)}
                      </span>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      <span className="font-medium text-foreground">
                        {meta.title}
                      </span>
                      {" - "}
                      {meta.description}
                    </p>
                  </div>
                );
              })}
            </div>
          </div>

          <div className="space-y-3 rounded-lg border border-border bg-muted/30 px-4 py-3">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <p className="text-xs uppercase tracking-wide text-text-tertiary">
                  Service access
                </p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {serviceAccess.allow_all_services
                    ? "This app will be able to use all of your available services through the proxy."
                    : summaryServices.length > 0
                      ? "This app will be able to use these services through the proxy:"
                      : "No service access requested. This app only signs you in."}
                </p>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="shrink-0"
                onClick={() => setCustomize((current) => !current)}
              >
                {customize ? "Done" : "Customize"}
              </Button>
            </div>

            {!customize &&
              !serviceAccess.allow_all_services &&
              (summaryServices.length > 0 || unmatchedDefaults.length > 0) && (
                <div className="space-y-2 border-t border-border pt-3">
                  {summaryServices.map((item) => (
                    <div
                      key={item.id}
                      className="flex items-start justify-between gap-2 rounded-md border border-border bg-background/60 px-3 py-2"
                    >
                      <div className="min-w-0">
                        <p className="break-words text-xs font-medium text-foreground">
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
                      {item.requestedByApp && (
                        <Badge
                          variant="secondary"
                          className="shrink-0 text-[10px]"
                        >
                          Requested by app
                        </Badge>
                      )}
                    </div>
                  ))}
                  {unmatchedDefaults.map((name) => (
                    <div
                      key={`unmatched-${name}`}
                      className="rounded-md border border-dashed border-border bg-background/40 px-3 py-2"
                    >
                      <p className="break-words text-xs text-muted-foreground">
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
              <div className="space-y-3 border-t border-border pt-3">
                <div className="flex items-center justify-between gap-2">
                  <Label htmlFor="oauth-allow-all-services">All services</Label>
                  <Switch
                    id="oauth-allow-all-services"
                    aria-label="All services"
                    checked={allowAllServices}
                    onCheckedChange={setAllowAllServices}
                  />
                </div>

                {!allowAllServices && (
                  <div className="space-y-2">
                    {userServicesLoading ? (
                      <p className="text-xs text-muted-foreground">
                        Loading services...
                      </p>
                    ) : selectableServices.length > 0 ? (
                      selectableServices.map((service) => {
                        const orgName = serviceOrgName(service);
                        return (
                          <div
                            key={service.id}
                            className="flex items-start gap-2 rounded-md border border-border bg-background/60 px-3 py-2"
                          >
                            <Checkbox
                              id={`oauth-service-${service.id}`}
                              checked={effectiveSelectedServiceIds.includes(
                                service.id,
                              )}
                              onCheckedChange={(checked) =>
                                toggleService(service.id, checked === true)
                              }
                            />
                            <div className="min-w-0">
                              <Label
                                htmlFor={`oauth-service-${service.id}`}
                                className="cursor-pointer text-xs leading-5 text-foreground"
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
                            </div>
                          </div>
                        );
                      })
                    ) : (
                      <p className="text-xs text-muted-foreground">
                        No active services are available.
                      </p>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Client ID
            </p>
            <p className="text-xs text-foreground">{clientId}</p>
          </div>

          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Redirect URI
            </p>
            <p className="break-all text-xs text-foreground">{redirectUri}</p>
          </div>

          <div className="rounded-lg border border-yellow-500/30 bg-yellow-500/10 px-4 py-3 light:border-warning/40 light:bg-warning/10">
            <div className="flex items-start gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 text-yellow-300 light:text-warning" />
              <p className="text-xs text-yellow-100/90 light:text-warning">
                Only continue if you trust this application. You can revoke this
                access later from <strong>Authorized Applications</strong>.
              </p>
            </div>
          </div>

          <form
            method="POST"
            action="/oauth/authorize/decision"
            className="flex flex-wrap items-center gap-3 pt-1"
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
            {resources.length > 0 && (
              <input
                type="hidden"
                name="resource_selection_present"
                value="true"
              />
            )}
            {selectedResources.map((resource) => (
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
            >
              Deny
            </Button>
            <Button
              variant="primary"
              type="submit"
              name="decision"
              value="allow"
            >
              Allow
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
