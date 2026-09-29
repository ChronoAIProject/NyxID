import { useEffect, useState } from "react";
import { Check, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUserServices } from "@/hooks/use-user-services";
import { OAUTH_SCOPE_META } from "@/lib/constants";
import type { UserServiceResponse } from "@/schemas/keys";
import type { IncrementalConsentRequest } from "@/schemas/oauth-consent";

function ServiceIdentity({
  service,
}: {
  readonly service: UserServiceResponse;
}) {
  const source = service.credential_source;
  return (
    <span className="block min-w-0">
      <span className="block break-words text-[13px] font-semibold text-foreground">
        {service.label || service.catalog_service_name || service.slug}
      </span>
      <span className="mt-1 block break-all font-mono text-[11px] font-normal text-muted-foreground">
        {service.slug}
      </span>
      <span className="mt-1 block text-[11px] font-normal text-muted-foreground">
        {source.type === "org"
          ? `Organization · ${source.org_name}`
          : "Personal"}
      </span>
    </span>
  );
}

export function OAuthIncrementalConsentPage({
  request,
  error,
}: {
  readonly request?: IncrementalConsentRequest;
  readonly error?: string;
}) {
  useApplyTheme();
  const { data: services, isLoading, isError } = useUserServices();
  const [optionalIds, setOptionalIds] = useState<readonly string[]>([]);
  const [expired, setExpired] = useState(() =>
    request ? request.exp * 1000 <= Date.now() : false,
  );
  useEffect(() => {
    if (!request) return;
    const timeout = window.setTimeout(
      () => setExpired(true),
      Math.max(0, request.exp * 1000 - Date.now()),
    );
    return () => window.clearTimeout(timeout);
  }, [request]);
  const snapshot = request?.incremental_consent;
  const available = (services ?? []).filter(
    (service) =>
      service.is_active &&
      (service.credential_source.type === "personal" ||
        service.credential_source.allowed),
  );
  const currentIds = [...new Set(snapshot?.current_service_ids ?? [])];
  const requiredIds = [...new Set(snapshot?.required_service_ids ?? [])];
  const requiredAdditions = snapshot?.allow_all_services
    ? []
    : requiredIds.filter((id) => !currentIds.includes(id));
  const additions = [...new Set([...requiredAdditions, ...optionalIds])];
  const currentScopes = snapshot?.current_scopes.split(/\s+/).filter(Boolean) ?? [];
  const addedScopes = snapshot?.scopes
    .split(/\s+/)
    .filter((scope) => scope && !currentScopes.includes(scope)) ?? [];
  const unavailable = [
    ...new Set([...requiredAdditions, ...optionalIds]),
  ].filter((id) => !available.some((service) => service.id === id));
  const optional = snapshot?.allow_all_services
    ? []
    : available.filter(
        (service) =>
          !currentIds.includes(service.id) && !requiredIds.includes(service.id),
      );
  const needsServiceInventory = requiredAdditions.length > 0 || optionalIds.length > 0;
  const blocked = Boolean(
    error || expired || (needsServiceInventory && (isLoading || isError || unavailable.length)),
  );
  const errorMessage =
    error ||
    (expired
      ? "This authorization request has expired. Return to the application and try again."
      : isError && needsServiceInventory
        ? "Services could not be loaded. Reload this page to try again."
        : !isLoading && unavailable.length
          ? "A requested service is unavailable, disconnected, or no longer shared with you. Review application access and restart authorization."
          : undefined);

  function serviceRow(id: string, retained: boolean) {
    const service = available.find((item) => item.id === id);
    if (!service && !retained) return null;
    if (!service) {
      return (
        <div key={id} className="px-4 py-3 text-[12px] text-muted-foreground">
          {isLoading || isError
            ? "Previously authorized service details unavailable: "
            : "Previously authorized service unavailable: "}
          <span className="break-all font-mono">{id}</span>
        </div>
      );
    }
    return (
      <div key={id} className="flex items-start gap-3 px-4 py-3">
        <Check
          aria-hidden="true"
          className="mt-0.5 h-4 w-4 shrink-0 text-success"
        />
        <div className="min-w-0 flex-1">
          <ServiceIdentity service={service} />
        </div>
        <Badge variant={retained ? "secondary" : "info"}>
          {retained ? "Authorized" : "Required"}
        </Badge>
      </div>
    );
  }

  return (
    <main className="flex min-h-dvh justify-center bg-background px-4 py-8 text-foreground sm:py-10">
      <div className="flex w-full max-w-xl flex-col gap-5">
        <div className="flex justify-center">
          <NyxidLogo className="h-8 w-auto" />
        </div>
        <header className="space-y-2 text-center">
          <h1 className="text-[22px] font-bold leading-tight tracking-tight sm:text-[28px]">
            Choose what {snapshot?.client_name ?? "this application"} can access
          </h1>
          <p className="text-[12px] text-muted-foreground">
            Review the new services and permissions requested.
          </p>
        </header>
        {errorMessage && <ErrorBanner message={errorMessage} />}
        {request && snapshot && (
          <Card className="border-border/50">
            <CardContent className="space-y-4 pt-4">
              <div className="rounded-xl border border-border/50 bg-overlay p-4">
                <p className="break-words text-[15px] font-semibold">
                  {snapshot.client_name}
                </p>
                <p className="mt-1 break-all text-[12px] text-muted-foreground">
                  Return to {new URL(request.redirect_uri).host || new URL(request.redirect_uri).protocol.slice(0, -1)}
                </p>
              </div>
              {isLoading && needsServiceInventory ? (
                <p role="status" className="text-[12px] text-muted-foreground">
                  Loading services...
                </p>
              ) : (
                <>
                  <DetailSection
                    title={
                      additions.length
                        ? `Allow ${additions.length} additional ${additions.length === 1 ? "service" : "services"}`
                        : "No additional services needed"
                    }
                    className="bg-overlay"
                  >
                    <p className="px-4 py-3 text-[12px] leading-relaxed text-muted-foreground">
                      {requiredAdditions.length
                        ? "These services are required to continue. Cancel if you do not want to grant access."
                        : "The requested services are already included in this application's access."}
                    </p>
                    {requiredAdditions.map((id) => serviceRow(id, false))}
                    {optionalIds.map((id) => {
                      const service = available.find((item) => item.id === id);
                      return service ? (
                        <div
                          key={id}
                          className="flex items-start justify-between gap-3 px-4 py-3"
                        >
                          <ServiceIdentity service={service} />
                          <Badge variant="secondary">Optional</Badge>
                        </div>
                      ) : null;
                    })}
                    {optional.length > 0 && (
                      <details className="border-t border-border/50 px-4 py-3">
                        <summary className="cursor-pointer text-[12px] text-muted-foreground">
                          Add optional services
                        </summary>
                        <div className="mt-3 space-y-3">
                          {optional.map((service) => (
                            <div
                              key={service.id}
                              className="flex items-start gap-3"
                            >
                              <Checkbox
                                id={`incremental-${service.id}`}
                                checked={optionalIds.includes(service.id)}
                                onCheckedChange={(checked) =>
                                  setOptionalIds((ids) =>
                                    checked === true
                                      ? [...ids, service.id]
                                      : ids.filter((id) => id !== service.id),
                                  )
                                }
                              />
                              <Label
                                htmlFor={`incremental-${service.id}`}
                                className="min-w-0 cursor-pointer"
                              >
                                <ServiceIdentity service={service} />
                              </Label>
                            </div>
                          ))}
                        </div>
                      </details>
                    )}
                  </DetailSection>
                  {addedScopes.length > 0 && (
                    <DetailSection title={`Allow ${addedScopes.length} additional ${addedScopes.length === 1 ? "permission" : "permissions"}`} className="bg-overlay">
                      {addedScopes.map((scope) => (
                        <div key={scope} className="px-4 py-3 text-[12px]">
                          <p className="font-semibold text-foreground">{OAUTH_SCOPE_META[scope]?.title ?? scope}</p>
                          {OAUTH_SCOPE_META[scope]?.description && <p className="mt-1 text-muted-foreground">{OAUTH_SCOPE_META[scope].description}</p>}
                        </div>
                      ))}
                    </DetailSection>
                  )}
                  <DetailSection
                    title="Already authorized"
                    className="bg-overlay"
                  >
                    <p className="px-4 py-3 text-[12px] leading-relaxed text-muted-foreground">
                      {snapshot.allow_all_services
                        ? "This application already has access to all available services. This access will be retained."
                        : "Your existing access will be retained. You can manage it separately in Authorized Applications."}
                    </p>
                    {!snapshot.allow_all_services &&
                      currentIds.map((id) => serviceRow(id, true))}
                    {!snapshot.allow_all_services &&
                      currentIds.length === 0 && (
                        <p className="px-4 pb-3 text-[12px] text-muted-foreground">
                          No services previously authorized.
                        </p>
                      )}
                  </DetailSection>
                </>
              )}
              <details className="rounded-xl border border-border/50 px-4 py-3">
                <summary className="cursor-pointer text-[12px] text-muted-foreground">
                  Application permissions and details
                </summary>
                <ul className="mt-3 space-y-2 text-[12px] text-muted-foreground">
                  {currentScopes.map((scope) => (
                    <li key={scope}>
                      <span className="text-foreground">
                        {OAUTH_SCOPE_META[scope]?.title ?? scope}
                      </span>
                      {OAUTH_SCOPE_META[scope]?.description && (
                        <p>{OAUTH_SCOPE_META[scope].description}</p>
                      )}
                    </li>
                  ))}
                </ul>
                <p className="mt-3 break-all font-mono text-[11px] text-muted-foreground">
                  Client ID: {request.client_id}
                </p>
                <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                  {request.redirect_uri}
                </p>
              </details>
              <p className="text-[12px] text-muted-foreground">
                Only continue if you trust this application.
              </p>
              <form
                method="POST"
                action="/oauth/authorize/incremental/decision"
                className="flex flex-col gap-2 sm:flex-row sm:justify-end"
              >
                <input
                  type="hidden"
                  name="consent_request"
                  value={request.token}
                />
                <input
                  type="hidden"
                  name="response_type"
                  value={request.response_type}
                />
                <input
                  type="hidden"
                  name="client_id"
                  value={request.client_id}
                />
                <input
                  type="hidden"
                  name="redirect_uri"
                  value={request.redirect_uri}
                />
                <input type="hidden" name="scope" value={request.scope} />
                <input type="hidden" name="state" value={request.state ?? ""} />
                <input
                  type="hidden"
                  name="code_challenge"
                  value={request.code_challenge}
                />
                <input
                  type="hidden"
                  name="code_challenge_method"
                  value={request.code_challenge_method}
                />
                <input
                  type="hidden"
                  name="service_access_mode"
                  value="incremental"
                />
                {(
                  [
                    "nonce",
                    "binding_grant_id",
                    "external_subject_platform",
                    "external_subject_tenant",
                    "external_subject_external_user_id",
                  ] as const
                ).map((name) =>
                  request[name] ? (
                    <input
                      key={name}
                      type="hidden"
                      name={name}
                      value={request[name]}
                    />
                  ) : null,
                )}
                {request.requested_service_ids.map((id, index) => (
                  <input
                    key={`${id}-${index}`}
                    type="hidden"
                    name="requested_service_ids"
                    value={id}
                  />
                ))}
                <input
                  type="hidden"
                  name="allow_all_services"
                  value={snapshot.allow_all_services ? "true" : "false"}
                />
                {[...new Set([...currentIds, ...additions])].map((id) => (
                  <input
                    key={id}
                    type="hidden"
                    name="allowed_service_ids"
                    value={id}
                  />
                ))}
                {request.resource.map((resource) => (
                  <input
                    key={resource}
                    type="hidden"
                    name="resource"
                    value={resource}
                  />
                ))}
                <Button
                  type="submit"
                  name="decision"
                  value="deny"
                  variant="outline"
                >
                  {expired ? "Return to application" : "Cancel"}
                </Button>
                <Button
                  type="submit"
                  name="decision"
                  value="allow"
                  variant="primary"
                  disabled={blocked}
                >
                  <ButtonIcon variant="primary">
                    <ShieldCheck />
                  </ButtonIcon>
                  {additions.length || addedScopes.length
                    ? `Allow ${[
                        additions.length ? `${additions.length} ${additions.length === 1 ? "service" : "services"}` : "",
                        addedScopes.length ? `${addedScopes.length} ${addedScopes.length === 1 ? "permission" : "permissions"}` : "",
                      ].filter(Boolean).join(" and ")}`
                    : "Continue"}
                </Button>
              </form>
            </CardContent>
          </Card>
        )}
      </div>
    </main>
  );
}
