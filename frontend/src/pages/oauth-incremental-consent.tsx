import { useEffect, useState } from "react";
import { AlertTriangle, ArrowRight, Check, ChevronDown } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { DetailRow } from "@/components/shared/detail-row";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useApplyTheme } from "@/hooks/use-theme";
import { useUserServices } from "@/hooks/use-user-services";
import { OAUTH_SCOPE_META } from "@/lib/constants";
import type { UserServiceResponse } from "@/schemas/keys";
import type { IncrementalConsentRequest } from "@/schemas/oauth-consent";
import { useAuthStore } from "@/stores/auth-store";

function ServiceIdentity({
  service,
}: {
  readonly service: UserServiceResponse;
}) {
  const source = service.credential_source;
  return (
    <span className="block min-w-0">
      <span className="block break-words text-[13px] font-medium text-foreground">
        {service.label || service.catalog_service_name || service.slug}
      </span>
      {service.catalog_service_description && (
        <span className="mt-1 line-clamp-2 block break-words text-[12px] leading-relaxed text-muted-foreground">
          {service.catalog_service_description}
        </span>
      )}
      <span className="mt-1 block break-all text-[11px] font-normal text-text-tertiary">
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
  const email = useAuthStore((state) => state.user?.email);
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
  const currentScopes =
    snapshot?.current_scopes.split(/\s+/).filter(Boolean) ?? [];
  const addedScopes =
    snapshot?.scopes
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
  const needsServiceInventory =
    requiredAdditions.length > 0 || optionalIds.length > 0;
  const blocked = Boolean(
    error ||
    expired ||
    (needsServiceInventory && (isLoading || isError || unavailable.length)),
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
        <div key={id} className="py-3.5 text-[12px] text-muted-foreground">
          {isLoading || isError
            ? "Previously authorized service details unavailable: "
            : "Previously authorized service unavailable: "}
          <span className="break-all font-mono">{id}</span>
        </div>
      );
    }
    return (
      <div key={id} className="flex items-start gap-3 py-3.5">
        <Check
          aria-hidden="true"
          className="mt-0.5 h-4 w-4 shrink-0 text-success"
        />
        <div className="min-w-0 flex-1">
          <ServiceIdentity service={service} />
        </div>
        <Badge
          variant={retained ? "secondary" : "accent"}
          className="shrink-0 text-[10px]"
        >
          {retained ? "Authorized" : "Required"}
        </Badge>
      </div>
    );
  }

  return (
    <main
      className="connect-link-shell flex min-h-dvh items-center justify-center px-4 py-6 text-foreground"
      style={{
        paddingTop: "max(1.25rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className="w-full max-w-[560px]">
        <header className="mb-4 flex flex-col items-center gap-1.5 text-center">
          <div className="mb-1 flex flex-col items-center gap-1">
            <div className="connection-identity-icon flex size-16 items-center justify-center rounded-full border border-border bg-card">
              <NyxidIcon className="size-8" />
            </div>
            <span className="text-xs font-medium text-foreground">NyxID</span>
          </div>
          <h1 className="break-words text-[22px] font-bold leading-tight text-foreground sm:text-[28px]">
            Update service access
          </h1>
          <p className="max-w-md break-words text-[12px] leading-relaxed text-muted-foreground">
            <span className="font-medium text-foreground">
              {snapshot?.client_name ?? "This application"}
            </span>{" "}
            is requesting additional access to your NyxID account.
          </p>
          {request && (
            <p className="break-all text-[11px] text-muted-foreground">
              Return to{" "}
              {new URL(request.redirect_uri).host ||
                new URL(request.redirect_uri).protocol.slice(0, -1)}
            </p>
          )}
        </header>
        <div className="connect-link-card overflow-hidden rounded-xl border border-border/70 bg-card shadow-sm">
          <div className="p-5 sm:p-8">
            {errorMessage && <ErrorBanner message={errorMessage} />}
            {request && snapshot && (
              <>
                {isLoading && needsServiceInventory ? (
                  <p
                    role="status"
                    className="py-5 text-[12px] text-muted-foreground"
                  >
                    Loading services...
                  </p>
                ) : (
                  <>
                    <section
                      aria-labelledby="incremental-services"
                      className="pb-5"
                    >
                      <h2
                        id="incremental-services"
                        className="text-[15px] font-semibold text-foreground"
                      >
                        {additions.length
                          ? `Allow ${additions.length} additional ${additions.length === 1 ? "service" : "services"}`
                          : "No additional services needed"}
                      </h2>
                      <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">
                        {requiredAdditions.length
                          ? "These services are required to continue. Cancel if you do not want to grant access."
                          : "The requested services are already included in this application's access."}
                      </p>
                      {requiredAdditions.length > 0 && (
                        <div
                          role="region"
                          aria-label="Additional services"
                          tabIndex={0}
                          className="mt-4 max-h-[min(18rem,45dvh)] divide-y divide-border/60 overflow-y-auto overscroll-contain border-y border-border/60 focus-visible:outline-2 focus-visible:outline-ring"
                        >
                          {requiredAdditions.map((id) => serviceRow(id, false))}
                        </div>
                      )}
                      {optionalIds.map((id) => {
                        const service = available.find(
                          (item) => item.id === id,
                        );
                        return service ? (
                          <div
                            key={id}
                            className="flex items-start justify-between gap-3 border-b border-border/60 py-3.5"
                          >
                            <ServiceIdentity service={service} />
                            <Badge
                              variant="accent"
                              className="shrink-0 text-[10px]"
                            >
                              Optional
                            </Badge>
                          </div>
                        ) : null;
                      })}
                      {optional.length > 0 && (
                        <details className="group mt-4">
                          <summary className="flex cursor-pointer list-none items-center justify-between text-[12px] font-medium text-muted-foreground [&::-webkit-details-marker]:hidden">
                            Add optional services{" "}
                            <ChevronDown
                              className="h-4 w-4 transition-transform group-open:rotate-180"
                              aria-hidden="true"
                            />
                          </summary>
                          <div className="mt-3 max-h-[min(16rem,35dvh)] divide-y divide-border/60 overflow-y-auto border-y border-border/60">
                            {optional.map((service) => (
                              <div
                                key={service.id}
                                className="flex items-start gap-3 py-3.5"
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
                    </section>
                    {addedScopes.length > 0 && (
                      <section
                        aria-labelledby="incremental-permissions"
                        className="border-t border-border py-5"
                      >
                        <h2
                          id="incremental-permissions"
                          className="text-[15px] font-semibold text-foreground"
                        >
                          Allow {addedScopes.length} additional{" "}
                          {addedScopes.length === 1
                            ? "permission"
                            : "permissions"}
                        </h2>
                        <div className="mt-4 divide-y divide-border/60 border-y border-border/60">
                          {addedScopes.map((scope) => (
                            <div key={scope} className="py-3.5 text-[12px]">
                              <p className="font-medium text-foreground">
                                {OAUTH_SCOPE_META[scope]?.title ?? scope}
                              </p>
                              {OAUTH_SCOPE_META[scope]?.description && (
                                <p className="mt-1 leading-relaxed text-muted-foreground">
                                  {OAUTH_SCOPE_META[scope].description}
                                </p>
                              )}
                            </div>
                          ))}
                        </div>
                      </section>
                    )}
                    <section
                      aria-labelledby="incremental-existing"
                      className="border-t border-border py-5"
                    >
                      <h2
                        id="incremental-existing"
                        className="text-[15px] font-semibold text-foreground"
                      >
                        Already authorized
                      </h2>
                      <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">
                        {snapshot.allow_all_services
                          ? "This application already has access to all available services. This access will be retained."
                          : "Your existing access will be retained. You can manage it separately in Authorized Applications."}
                      </p>
                      {!snapshot.allow_all_services &&
                        currentIds.length > 0 && (
                          <div
                            role="region"
                            aria-label="Already authorized services"
                            tabIndex={0}
                            className="mt-4 max-h-[min(18rem,45dvh)] divide-y divide-border/60 overflow-y-auto overscroll-contain border-y border-border/60 focus-visible:outline-2 focus-visible:outline-ring"
                          >
                            {currentIds.map((id) => serviceRow(id, true))}
                          </div>
                        )}
                      {!snapshot.allow_all_services &&
                        currentIds.length === 0 && (
                          <p className="mt-3 text-[12px] text-muted-foreground">
                            No services previously authorized.
                          </p>
                        )}
                    </section>
                  </>
                )}
                <section className="border-t border-border py-5">
                  <div className="flex items-start gap-3">
                    <AlertTriangle
                      className="mt-0.5 h-4 w-4 shrink-0 text-warning"
                      aria-hidden="true"
                    />
                    <div>
                      <h2 className="text-[14px] font-semibold text-foreground">
                        Make sure you trust {snapshot.client_name}
                      </h2>
                      <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">
                        This app may receive the account information above and
                        use the services you approve. You can revoke access
                        later from Authorized Applications.
                      </p>
                    </div>
                  </div>
                </section>
                <details className="group border-t border-border py-4">
                  <summary className="flex cursor-pointer list-none items-center justify-between text-[12px] font-medium text-muted-foreground [&::-webkit-details-marker]:hidden">
                    App details{" "}
                    <ChevronDown
                      className="h-4 w-4 transition-transform group-open:rotate-180"
                      aria-hidden="true"
                    />
                  </summary>
                  <div className="mt-3 divide-y divide-border/60 border-y border-border/60">
                    <DetailRow
                      label="Application"
                      value={snapshot.client_name}
                    />
                    <DetailRow
                      label="Redirect host"
                      value={new URL(request.redirect_uri).host}
                    />
                    <DetailRow
                      label="Client ID"
                      value={request.client_id}
                      mono
                      copyable
                    />
                    <DetailRow
                      label="Redirect URI"
                      value={request.redirect_uri}
                      mono
                      copyable
                    />
                    <DetailRow
                      label="Existing scopes"
                      value={currentScopes.join(" ")}
                      mono
                    />
                  </div>
                </details>
                <form
                  method="POST"
                  action="/oauth/authorize/incremental/decision"
                  className="flex flex-col gap-2 border-t border-border pt-5"
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
                  <input
                    type="hidden"
                    name="state"
                    value={request.state ?? ""}
                  />
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
                    value="allow"
                    variant="primary"
                    disabled={blocked}
                    className="w-full"
                  >
                    {additions.length || addedScopes.length
                      ? `Allow ${[
                          additions.length
                            ? `${additions.length} ${additions.length === 1 ? "service" : "services"}`
                            : "",
                          addedScopes.length
                            ? `${addedScopes.length} ${addedScopes.length === 1 ? "permission" : "permissions"}`
                            : "",
                        ]
                          .filter(Boolean)
                          .join(" and ")}`
                      : "Continue"}
                    <ArrowRight className="ml-2 size-4" aria-hidden="true" />
                  </Button>
                  <Button
                    type="submit"
                    name="decision"
                    value="deny"
                    variant="ghost"
                    className="w-full"
                  >
                    {expired ? "Return to application" : "Cancel"}
                  </Button>
                </form>
              </>
            )}
          </div>
          <div className="border-t border-border px-5 py-3 text-center text-xs text-muted-foreground sm:px-8">
            Signed in as{" "}
            <span className="break-all text-foreground">
              {email || "your NyxID account"}
            </span>
          </div>
        </div>
        <p className="mt-5 text-center text-xs text-muted-foreground">
          Application access via NyxID
        </p>
      </div>
    </main>
  );
}
