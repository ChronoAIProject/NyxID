import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { MachineGrantConsent } from "@/components/shared/machine-grant-consent";
import { readMachineConsentScope } from "@/schemas/machine-grants";
import { readProtectedConsentRequest } from "@/hooks/use-machine-grants";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label, FieldHelp } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  AlertTriangle,
  ArrowRight,
  ChevronDown,
  CircleUserRound,
  KeyRound,
  Mail,
  RefreshCw,
  Save,
  ShieldCheck,
  ShieldQuestion,
  UserRound,
  UsersRound,
  Waypoints,
} from "lucide-react";
import { OAUTH_SCOPE_META } from "@/lib/constants";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { DetailRow } from "@/components/shared/detail-row";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useUserServices } from "@/hooks/use-user-services";
import { cn } from "@/lib/utils";
import {
  oauthConsentServiceAccessSchema,
  readIncrementalConsentRequest,
} from "@/schemas/oauth-consent";
import { useAuthStore } from "@/stores/auth-store";

import {
  agentGrant,
  type AccessLevel,
  type AccessServiceRow,
} from "@/lib/agent-grant";
import { ServiceAccessTable } from "./oauth-consent-access";
import { OAuthIncrementalConsentPage } from "./oauth-incremental-consent";

// Paint the canvas because public pages leave a dark body anti-flash color.
function ConsentShell({
  children,
  email,
  heading,
  preview = false,
  wide = false,
}: {
  readonly children: ReactNode;
  readonly email?: string;
  readonly heading?: ReactNode;
  readonly preview?: boolean;
  /** Two-column layout on large screens. */
  readonly wide?: boolean;
}) {
  useUiTranslation();
  return (
    <main
      className="connect-link-shell flex min-h-dvh items-center justify-center px-4 py-6 text-foreground"
      style={{
        paddingTop: "max(1.25rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className={cn("w-full max-w-[560px]", wide && "lg:max-w-[1040px]")}>
        {heading}
        <div className="connect-link-card overflow-hidden rounded-xl border border-border/70 bg-card shadow-sm">
          <div className="p-5 sm:p-8">{children}</div>
          <div className="border-t border-border px-5 py-3 text-center text-13 text-muted-foreground sm:px-8">
            {preview ? translateUi("Preview · ") : ""}{translateUi("Signed in as")}{" "}
            <span className="break-all text-foreground">
              {email || translateUi("your NyxID account")}
            </span>
          </div>
        </div>
        <p className="mt-5 text-center text-13 text-muted-foreground">{translateUi("Application access via NyxID")}</p>
      </div>
    </main>
  );
}

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

export interface ConsentServiceDisplay {
  readonly id: string;
  readonly is_active: boolean;
  readonly label?: string | null;
  readonly slug: string;
  readonly resource_uri: string;
  readonly catalog_service_name?: string | null;
  readonly catalog_service_description?: string | null;
  readonly credential_source: {
    readonly type: "personal" | "org";
    readonly org_name?: string;
    readonly allowed?: boolean;
  };
}

export interface ConsentPreview {
  readonly search: URLSearchParams;
  readonly services: readonly ConsentServiceDisplay[];
  readonly email: string;
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

export function OAuthConsentPage({
  preview,
}: {
  readonly preview?: ConsentPreview;
} = {}) {
  useUiTranslation();
  const [requestHandle] = useState(() =>
    new URLSearchParams(window.location.search).get("consent_request_id"),
  );
  const [incremental] = useState(() =>
    readIncrementalConsentRequest(new URLSearchParams(window.location.search)),
  );
  if (requestHandle) return <StoredIncrementalConsent handle={requestHandle} />;
  if (incremental) return <OAuthIncrementalConsentPage {...incremental} />;
  return <StandardConsentPage preview={preview} />;
}

function StoredIncrementalConsent({ handle }: { readonly handle: string }) {
  useUiTranslation();
  const owner = useAuthStore((state) => state.user?.id);
  const [request, setRequest] = useState<{ token?: string; owner?: string; failed?: boolean }>({});
  useEffect(() => {
    if (!owner) return;
    const abort = new AbortController();
    void readProtectedConsentRequest(handle, abort.signal).then((token) => {
      if (!abort.signal.aborted) setRequest({ token, owner });
    }).catch(() => {
      if (!abort.signal.aborted) setRequest({ failed: true, owner });
    });
    return () => abort.abort();
  }, [handle, owner]);
  const isPending = !request.token && !request.failed || request.owner !== owner;
  const isError = request.failed === true;
  if (isPending || isError) {
    return (
      <main className="mx-auto flex min-h-dvh max-w-xl items-center px-4">
        {isPending ? (
          <p role="status">{translateUi("Loading authorization request...")}</p>
        ) : (
          <ErrorBanner message={translateUi("This authorization request is unavailable. Return to the application and try again.")} />
        )}
      </main>
    );
  }
  const parsed = readIncrementalConsentRequest(
    new URLSearchParams({ consent_request: request.token ?? "" }),
  );
  return parsed ? (
    <OAuthIncrementalConsentPage {...parsed} />
  ) : (
    <OAuthIncrementalConsentPage error={translateUi("Invalid consent request. Please restart authorization.")} />
  );
}

const SCOPE_ICONS: Readonly<Record<string, typeof CircleUserRound>> = {
  openid: CircleUserRound,
  profile: UserRound,
  email: Mail,
  roles: ShieldCheck,
  groups: UsersRound,
  proxy: Waypoints,
  offline_access: RefreshCw,
  "urn:nyxid:scope:broker_binding": KeyRound,
};

const IDENTITY_SCOPE_WORDS: Readonly<Record<string, string>> = {
  openid: "identity",
  profile: "name",
  email: "email",
  roles: "roles",
  groups: "groups",
};

function identityScopeSummary(identityScopes: readonly string[]): string {
  const words = identityScopes.map((item) => IDENTITY_SCOPE_WORDS[item]);
  const list =
    words.length > 1
      ? `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`
      : (words[0] ?? "");
  return `See your ${list}.`;
}

function ScopeIcon({ scope }: { readonly scope: string }) {
  useUiTranslation();
  const Icon = SCOPE_ICONS[scope] ?? ShieldQuestion;
  return <Icon className="h-4 w-4" aria-hidden="true" />;
}

function ServiceScrollList({
  children,
  bordered = true,
}: {
  readonly children: ReactNode;
  readonly bordered?: boolean;
}) {
  useUiTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [hiddenEdges, setHiddenEdges] = useState({ top: false, bottom: false });
  const updateEdges = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    const top = element.scrollTop > 1;
    const bottom =
      element.scrollTop + element.clientHeight < element.scrollHeight - 1;
    setHiddenEdges((current) =>
      current.top === top && current.bottom === bottom
        ? current
        : { top, bottom },
    );
  }, []);

  useLayoutEffect(() => {
    updateEdges();
    const element = scrollRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateEdges);
    observer.observe(element);
    if (element.firstElementChild) observer.observe(element.firstElementChild);
    return () => observer.disconnect();
  }, [children, updateEdges]);

  return (
    <div className={`relative ${bordered ? "border-y border-border/60" : ""}`}>
      <div
        ref={scrollRef}
        tabIndex={0}
        role="region"
        aria-label={translateUi("Service access list")}
        onScroll={updateEdges}
        className="max-h-[min(18rem,45dvh)] overflow-y-auto overscroll-contain focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
      >
        <div className="divide-y divide-border/60">{children}</div>
      </div>
      <div
        aria-hidden="true"
        data-scroll-fade="top"
        className={`pointer-events-none absolute inset-x-0 top-0 h-7 bg-gradient-to-b from-card to-transparent transition-opacity duration-150 ${hiddenEdges.top ? "opacity-100" : "opacity-0"}`}
      />
      <div
        aria-hidden="true"
        data-scroll-fade="bottom"
        className={`pointer-events-none absolute inset-x-0 bottom-0 h-7 bg-gradient-to-t from-card to-transparent transition-opacity duration-150 ${hiddenEdges.bottom ? "opacity-100" : "opacity-0"}`}
      />
    </div>
  );
}

function StandardConsentPage({
  preview,
}: {
  readonly preview?: ConsentPreview;
} = {}) {
  useUiTranslation();
  useApplyTheme();
  const user = useAuthStore((state) => state.user);
  const { data: userServices, isLoading: userServicesLoading } =
    useUserServices(!preview);
  // The consent page renders once per authorize redirect; capture every
  // repeated query value in one immutable snapshot so downstream memos keep
  // stable array identities across local state updates.
  const [authorizeQuery] = useState(() => {
    const search =
      preview?.search ?? new URLSearchParams(window.location.search);
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
  const [machineSelectionRequired, setMachineSelectionRequired] = useState(false);
  const [preparedMachineRequest, setPreparedMachineRequest] = useState<string | null>(null);
  const machineScope = scope.split(/\s+/).includes("machine") || Boolean(readMachineConsentScope(consentRequest)?.scope.split(/\s+/).includes("machine"));
  // Server-resolved hints: the app's declared default services matched to
  // this user (fixed on the consent screen), and declared services the
  // user has no match for (informational only).
  const bindingReview =
    search.get("binding_review") === "true" && Boolean(bindingGrantId);
  const currentBindingAllowsAllServices =
    search.get("current_binding_allow_all_services") === "true";
  const isLarkBinding = externalSubjectPlatform.toLowerCase() === "lark";
  // Agent apps requesting `proxy` choose service grants; binding reviews
  // keep their own grant flow.
  const agentAccess = !bindingReview && scope.split(/\s+/).includes("proxy");
  const [allowAllServices, setAllowAllServices] = useState(
    bindingReview && currentBindingAllowsAllServices,
  );
  const [defaultLevel, setDefaultLevel] = useState<AccessLevel>("off");
  const [levelOverrides, setLevelOverrides] = useState<
    Readonly<Record<string, AccessLevel>>
  >(() => Object.fromEntries(preselectServiceIds.map((id) => [id, "allow"])));
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
  // Agent apps get one profile row; `proxy` is shown as the service table.
  const identityScopes = agentAccess
    ? scopes.filter((item) => IDENTITY_SCOPE_WORDS[item])
    : [];
  // Machine authority is reviewed separately in the optional selection below.
  const listedScopes = scopes.filter((item) => item !== "machine" &&
    (!agentAccess || (!IDENTITY_SCOPE_WORDS[item] && item !== "proxy")));
  const redirectHost = parseHost(redirectUri);
  const email = preview?.email ?? user?.email;
  const selectableServices = useMemo(
    () =>
      (preview?.services ?? userServices ?? [])
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
    [preview, userServices],
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
          ...preselectServiceIds,
          ...requiredServiceIds,
          ...resourceSelectedServiceIds,
          ...selectedServiceIds,
        ]),
      ).filter(
        (id) =>
          preselectServiceIds.includes(id) ||
          requiredServiceIds.includes(id) ||
          resourceSelectedServiceIds.includes(id) ||
          !deselectedServiceIds.includes(id),
      ),
    [
      deselectedServiceIds,
      preselectServiceIds,
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
          description: service?.catalog_service_description?.trim() || "",
          orgName: service ? serviceOrgName(service) : null,
          requiredByApp:
            preselectServiceIds.includes(id) ||
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

  const accessRows = useMemo<readonly AccessServiceRow[]>(
    () =>
      selectableServices.map((service) => ({
        id: service.id,
        name: serviceDisplayName(service),
        secondary: serviceSecondaryText(service),
        orgName: serviceOrgName(service),
        requiredByApp:
          resourceSelectedServiceIds.includes(service.id) ||
          requiredServiceIds.includes(service.id),
      })),
    [selectableServices, resourceSelectedServiceIds, requiredServiceIds],
  );
  const grant = agentGrant(accessRows, defaultLevel, levelOverrides);
  const submittedAccess = agentAccess
    ? {
        allow_all_services: grant.allowAll,
        allowed_service_ids: grant.serviceIds,
      }
    : serviceAccess;

  function changeDefaultLevel(level: AccessLevel) {
    setDefaultLevel(level);
    setLevelOverrides({});
  }

  function changeServiceLevel(serviceId: string, level: AccessLevel) {
    setLevelOverrides((current) => ({ ...current, [serviceId]: level }));
  }

  function toggleService(serviceId: string, checked: boolean) {
    if (
      !checked &&
      (preselectServiceIds.includes(serviceId) ||
        resourceSelectedServiceIds.includes(serviceId) ||
        requiredServiceIds.includes(serviceId))
    )
      return;
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
      <ConsentShell email={email} preview={Boolean(preview)}>
        <header className="py-12 text-center">
          <h1 className="type-compact-page-title text-foreground sm:type-page-title">{translateUi("Invalid consent request")}</h1>
        </header>
        <ErrorBanner message={translateUi("Missing required OAuth parameters. Please restart the sign-in flow.")} />
      </ConsentShell>
    );
  }

  return (
    <ConsentShell
      email={email}
      preview={Boolean(preview)}
      wide={agentAccess}
      heading={
        <header className="mb-4 flex flex-col items-center gap-1.5 text-center">
          <div className="mb-1 flex flex-col items-center gap-1">
            <div className="connection-identity-icon flex size-16 items-center justify-center rounded-full border border-border bg-card">
              <NyxidIcon className="size-8" />
            </div>
            <span className="type-secondary-emphasis text-foreground">NyxID</span>
          </div>
          <h1 className="break-words type-compact-page-title text-foreground sm:type-page-title">
            {bindingReview
              ? isLarkBinding
                ? translateUi("Review Lark bot access")
                : translateUi("Review application access")
              : isLarkBinding
                ? translateUi("Authorize Lark bot")
                : translateUi("Authorize application")}
          </h1>
          <p className="max-w-md break-words text-14 text-muted-foreground">
            {bindingReview ? (
              <>{translateUi("Review the NyxID services available to")}{" "}
                <span className="font-medium text-foreground">
                  {clientName}
                </span>
                .
              </>
            ) : (
              <>
                <span className="font-medium text-foreground">
                  {clientName}
                </span>{" "}{translateUi("wants to access your account via OAuth.")}</>
            )}
          </p>
          {preview && (
            <p className="type-secondary-emphasis uppercase text-muted-foreground">{translateUi("Preview · Decisions disabled")}</p>
          )}
        </header>
      }
    >
      <div
        className={cn(
          agentAccess &&
            "lg:grid lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)] lg:grid-rows-[auto_auto_1fr] lg:gap-x-10",
        )}
      >
        <section
          aria-labelledby="oauth-permissions"
          className={cn("pb-5", agentAccess && "lg:col-start-1 lg:row-start-1")}
        >
          <h2
            id="oauth-permissions"
            className="type-card-title text-foreground"
          >{translateUi("This will allow ")}{clientName}{translateUi(" to:")}</h2>
          <div className="mt-4 divide-y divide-border/60">
            {identityScopes.length > 0 && (
              <div className="flex items-start gap-3 py-3">
                <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-overlay text-muted-foreground">
                  <ScopeIcon scope="openid" />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="type-card-title text-foreground">{translateUi("Your NyxID profile")}</p>
                  <p className="mt-1 text-14 text-muted-foreground">
                    {identityScopeSummary(identityScopes)}
                  </p>
                  <code className="mt-1 block break-all text-14 text-muted-foreground">
                    {identityScopes.join(" ")}
                  </code>
                </div>
              </div>
            )}
            {listedScopes.map((item) => {
              const meta = OAUTH_SCOPE_META[item];
              return (
                <div
                  key={`meta-${item}`}
                  className="flex items-start gap-3 py-3.5"
                >
                  <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-overlay text-muted-foreground">
                    <ScopeIcon scope={item} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p
                      className={`type-card-title text-foreground ${meta ? "break-words" : "break-all font-mono"}`}
                    >
                      {meta?.title ?? item}
                    </p>
                    {meta && (
                      <>
                        <p className="mt-1 text-14 text-muted-foreground">
                          {meta.description}
                        </p>
                        <code className="mt-1 block break-all text-14 text-muted-foreground">
                          {item}
                        </code>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </section>

        {agentAccess ? (
          <ServiceAccessTable
            clientName={clientName}
            rows={accessRows}
            loading={!preview && userServicesLoading}
            defaultLevel={defaultLevel}
            onDefaultLevelChange={changeDefaultLevel}
            overrides={levelOverrides}
            onLevelChange={changeServiceLevel}
            className="lg:col-start-2 lg:row-span-3 lg:row-start-1 lg:border-t-0 lg:pt-0"
          />
        ) : (
          <section
            aria-labelledby="oauth-services"
            className="border-t border-border py-5"
          >
            <div className="flex items-center justify-between gap-4">
              <h2
                id="oauth-services"
                className="type-card-title text-foreground"
              >{translateUi("Service access")}</h2>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="shrink-0"
                aria-expanded={customize}
                aria-controls="oauth-service-list"
                onClick={() => setCustomize((current) => !current)}
              >
                {customize ? translateUi("Done") : translateUi("Customize")}
              </Button>
            </div>
            <p className="mt-1.5 text-14 text-muted-foreground">
              {serviceAccess.allow_all_services
                ? bindingReview && currentBindingAllowsAllServices
                  ? translateUi("This binding currently authorizes all available services.")
                  : translateUi("This app will be able to use all of your available services through the proxy.")
                : summaryServices.length > 0
                  ? bindingReview
                    ? translateUi("Review the current grant and select any additional services.")
                    : translateUi("This app will be able to use these services through the proxy:")
                  : translateUi("No service access requested. This app only signs you in.")}
            </p>
            <div id="oauth-service-list" className="mt-4">
              {!customize &&
                !serviceAccess.allow_all_services &&
                (summaryServices.length > 0 ||
                  unmatchedDefaults.length > 0) && (
                  <ServiceScrollList>
                    {summaryServices.map((item) => (
                      <div
                        key={item.id}
                        className="flex flex-col gap-2 py-3.5 sm:flex-row sm:items-start sm:justify-between sm:gap-4"
                      >
                        <div className="min-w-0">
                          <p className="break-words type-card-title text-foreground">
                            {item.primary}
                          </p>
                          {item.description && (
                            <p
                              className="mt-1 line-clamp-2 break-words text-14 text-muted-foreground"
                              title={item.description}
                            >
                              {item.description}
                            </p>
                          )}
                          {item.secondary && (
                            <p className="mt-1 break-words text-13 text-text-tertiary">
                              {item.secondary}
                            </p>
                          )}
                          {item.orgName && (
                            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                              <Badge variant="secondary" className="text-12">{translateUi("Org")}</Badge>
                              <span className="break-words text-13 text-muted-foreground">
                                {item.orgName}
                              </span>
                            </div>
                          )}
                        </div>
                        <div className="flex shrink-0 flex-wrap gap-1 sm:justify-end">
                          {item.currentlyAuthorized && bindingReview && (
                            <Badge variant="secondary" className="text-12">{translateUi("Authorized now")}</Badge>
                          )}
                          {item.requiredByApp && (
                            <Badge variant="secondary" className="text-12">{translateUi("Required by app")}</Badge>
                          )}
                          {item.newlySelected && (
                            <Badge variant="accent" className="text-12">{translateUi("New")}</Badge>
                          )}
                        </div>
                      </div>
                    ))}
                    {unmatchedDefaults.map((name) => (
                      <div key={`unmatched-${name}`} className="py-3.5">
                        <p className="break-words text-14 text-muted-foreground">
                          <span className="font-medium text-foreground">
                            {name}
                          </span>{" "}{translateUi("— requested by this app, but you have no matching service in your account.")}</p>
                      </div>
                    ))}
                  </ServiceScrollList>
                )}

              {customize && (
                <div className="divide-y divide-border/60 border-y border-border/60">
                  <div className="flex items-center justify-between gap-3 py-3.5">
                    <Label htmlFor="oauth-allow-all-services">{translateUi("All services")}</Label>
                    <Switch
                      id="oauth-allow-all-services"
                      aria-label={translateUi("All services")}
                      checked={allowAllServices}
                      onCheckedChange={setAllowAllServices}
                    />
                  </div>

                  {!allowAllServices && (
                    <ServiceScrollList bordered={false}>
                      {!preview && userServicesLoading ? (
                        <p className="py-3.5 text-14 text-muted-foreground">{translateUi("Loading services...")}</p>
                      ) : selectableServices.length > 0 ? (
                        selectableServices.map((service) => {
                          const orgName = serviceOrgName(service);
                          const requiredByApp =
                            preselectServiceIds.includes(service.id) ||
                            resourceSelectedServiceIds.includes(service.id) ||
                            requiredServiceIds.includes(service.id);
                          return (
                            <div
                              key={service.id}
                              className="flex items-start gap-3 py-3.5"
                            >
                              <Checkbox
                                id={`oauth-service-${service.id}`}
                                checked={effectiveSelectedServiceIds.includes(
                                  service.id,
                                )}
                                disabled={requiredByApp}
                                onCheckedChange={(checked) =>
                                  toggleService(service.id, checked === true)
                                }
                              />
                              <div className="min-w-0">
                                <Label
                                  htmlFor={`oauth-service-${service.id}`}
                                  className={`${requiredByApp ? "cursor-default" : "cursor-pointer"} text-foreground`}
                                >
                                  <span className="block break-words">
                                    {serviceDisplayName(service)}
                                  </span>
                                  {service.catalog_service_description?.trim() && (
                                    <FieldHelp variant="body"
                                      className="mt-1 line-clamp-2 break-words text-muted-foreground"
                                      title={
                                        service.catalog_service_description
                                      }
                                    >
                                      {service.catalog_service_description}
                                    </FieldHelp>
                                  )}
                                  {serviceSecondaryText(service) && (
                                    <FieldHelp className="mt-1 block break-words text-text-tertiary">
                                      {serviceSecondaryText(service)}
                                    </FieldHelp>
                                  )}
                                </Label>
                                {orgName && (
                                  <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                                    <Badge
                                      variant="secondary"
                                      className="text-12"
                                    >{translateUi("Org")}</Badge>
                                    <span className="break-words type-secondary text-muted-foreground">
                                      {orgName}
                                    </span>
                                  </div>
                                )}
                                <div className="mt-1.5 flex flex-wrap gap-1">
                                  {bindingReview &&
                                    (currentBindingAllowsAllServices ||
                                      currentBindingServiceIds.includes(
                                        service.id,
                                      )) && (
                                      <Badge
                                        variant="secondary"
                                        className="text-12"
                                      >{translateUi("Authorized now")}</Badge>
                                    )}
                                  {requiredByApp && (
                                    <Badge
                                      variant="secondary"
                                      className="text-12"
                                    >{translateUi("Required by app")}</Badge>
                                  )}
                                  {bindingReview &&
                                    effectiveSelectedServiceIds.includes(
                                      service.id,
                                    ) &&
                                    !currentBindingAllowsAllServices &&
                                    !currentBindingServiceIds.includes(
                                      service.id,
                                    ) &&
                                    !requiredServiceIds.includes(
                                      service.id,
                                    ) && (
                                      <Badge
                                        variant="accent"
                                        className="text-12"
                                      >{translateUi("New")}</Badge>
                                    )}
                                </div>
                              </div>
                            </div>
                          );
                        })
                      ) : (
                        <p className="py-3.5 text-14 text-muted-foreground">{translateUi("No active services are available.")}</p>
                      )}
                    </ServiceScrollList>
                  )}
                  <div className="py-3">
                    <Button
                      type="button"
                      variant="secondary"
                      className="w-full"
                      onClick={() => setCustomize(false)}
                    >
                      <Save aria-hidden="true" />{translateUi("Save selection")}</Button>
                  </div>
                </div>
              )}
            </div>
          </section>
        )}

        <section
          aria-labelledby="oauth-trust"
          className={cn(
            "border-t border-border py-5",
            agentAccess && "lg:col-start-1 lg:row-start-2",
          )}
        >
          <div className="flex items-start gap-3">
            <AlertTriangle
              className="mt-0.5 h-4 w-4 shrink-0 text-warning"
              aria-hidden="true"
            />
            <div>
              <h2
                id="oauth-trust"
                className="type-body-strong text-foreground"
              >{translateUi("Make sure you trust ")}{clientName}
              </h2>
              <p className="mt-2 text-14 text-muted-foreground">{translateUi("This app may receive the account information above and use the services you approve. Continue only if you trust it. You can revoke access later from Authorized Applications.")}</p>
            </div>
          </div>
        </section>

        <details
          className={cn(
            "group border-t border-border py-4",
            agentAccess && "lg:col-start-1 lg:row-start-3 lg:self-start",
          )}
        >
          <summary className="flex cursor-pointer list-none items-center justify-between text-muted-foreground [&::-webkit-details-marker]:hidden">{translateUi("App details")}<ChevronDown
              className="h-4 w-4 transition-transform group-open:rotate-180"
              aria-hidden="true"
            />
          </summary>
          <div className="mt-3 divide-y divide-border/60 border-y border-border/60">
            <DetailRow label={translateUi("Application")} value={clientName} />
            <DetailRow label={translateUi("Redirect host")} value={redirectHost} />
            <DetailRow label={translateUi("Client ID")} value={clientId} mono copyable />
            <DetailRow label={translateUi("Redirect URI")} value={redirectUri} mono copyable />
            <DetailRow label={translateUi("Requested scopes")} value={scope} mono />
          </div>
        </details>
      </div>

      {machineScope && <MachineGrantConsent consentRequest={consentRequest} onPrepared={setPreparedMachineRequest} onSelectionRequired={setMachineSelectionRequired} disabled={Boolean(preview)} allowedServiceIds={submittedAccess.allow_all_services ? undefined : submittedAccess.allowed_service_ids} />}
      <form
        method="POST"
        action="/oauth/authorize/decision"
        onSubmit={preview ? (event) => event.preventDefault() : undefined}
        className={cn(
          "flex flex-col gap-2 border-t border-border pt-5",
          agentAccess && "lg:flex-row-reverse lg:items-center lg:justify-start",
        )}
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
        <input type="hidden" name="consent_request" value={preparedMachineRequest ?? consentRequest} />
        <input type="hidden" name="browser_form" value="true" />
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
          <input type="hidden" name="binding_grant_id" value={bindingGrantId} />
        )}
        <input
          type="hidden"
          name="allow_all_services"
          value={submittedAccess.allow_all_services ? "true" : "false"}
        />
        {!submittedAccess.allow_all_services &&
          submittedAccess.allowed_service_ids.map((serviceId) => (
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
          variant="primary"
          name="decision"
          value="allow"
          disabled={Boolean(preview) || (machineScope && machineSelectionRequired && !preparedMachineRequest)}
          className={cn("w-full", agentAccess && "lg:w-auto lg:min-w-44")}
        >
          {bindingReview ? translateUi("Update access") : translateUi("Allow access")}
          <ArrowRight className="ml-2 size-4" aria-hidden="true" />
        </Button>
        <Button
          type="submit"
          variant="ghost"
          name="decision"
          value="deny"
          disabled={Boolean(preview)}
          className={cn("w-full", agentAccess && "lg:w-auto lg:min-w-28")}
        >
          {bindingReview ? translateUi("Cancel") : translateUi("Decline")}
        </Button>
      </form>
    </ConsentShell>
  );
}
