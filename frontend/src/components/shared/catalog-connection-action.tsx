import { useApprovalAuthoringCapabilities } from "@/hooks/use-approvals";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import {
  useCatalogEntry,
  useKeys,
  useConnectCatalogService,
  useUpdateKey,
} from "@/hooks/use-keys";
import { useCreateAssistantConnectLink } from "@/hooks/use-connect-links";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";

/** The same persisted personal connection used by the NyxBot Plugins page. */
export function CatalogConnectionAction({
  slug,
  catalogEntry,
}: {
  readonly slug: string;
  readonly catalogEntry?: import("@/types/keys").CatalogEntry;
}) {
  useUiTranslation();
  const entryQuery = useCatalogEntry(catalogEntry ? null : slug);
  const entry = {
    ...entryQuery,
    data: catalogEntry ?? entryQuery.data,
    isLoading: !catalogEntry && entryQuery.isLoading,
  };
  const keys = useKeys();
  const connect = useConnectCatalogService();
  const capabilities = useApprovalAuthoringCapabilities();
  const update = useUpdateKey();
  const hosted = useCreateAssistantConnectLink();
  const rows =
    keys.data?.filter(
      (key) =>
        !key.auto_connected &&
        key.catalog_service_slug === slug &&
        key.credential_source?.type !== "org",
    ) ?? [];

  const pending = connect.isPending || update.isPending || hosted.isPending;
  const needsOptIn = Boolean(
    entry.data &&
    (entry.data.platform_key?.available ||
      (!entry.data.requires_credential && !entry.data.requires_gateway_url)),
  );
  const optInBlocked =
    needsOptIn &&
    (capabilities.data?.catalog_opt_in !== true || Boolean(capabilities.error));
  async function activate() {
    if (
      entry.data &&
      (entry.data.platform_key?.available ||
        (!entry.data.requires_credential && !entry.data.requires_gateway_url))
    ) {
      await connect.mutateAsync({ slug });
    } else {
      const href = await hosted.mutateAsync({
        serviceSlug: slug,
        personal: false,
      });
      window.location.assign(href);
    }
  }
  const error =
    entry.error ?? keys.error ?? connect.error ?? update.error ?? hosted.error;
  return (
    <div className="space-y-3">
      {!rows.length && optInBlocked && (
        <ErrorBanner
          message={translateUi(
            "Service connections are not ready yet. Try again shortly.",
          )}
          onRetry={() => void capabilities.refetch()}
        />
      )}
      {error && (
        <ErrorBanner
          message={
            error instanceof Error
              ? error.message
              : translateUi("Could not connect service")
          }
        />
      )}
      {rows.map((connection) => (
        <div
          key={connection.id}
          className="flex items-center justify-between gap-3"
        >
          <span className="type-row-title">{connection.label}</span>
          <Button
            variant="outline"
            disabled={pending}
            onClick={() => {
              void update
                .mutateAsync({
                  keyId: connection.id,
                  is_active: !connection.is_active,
                })
                .catch(() => undefined);
            }}
          >
            {translateUi(connection.is_active ? "Disable" : "Enable")}
          </Button>
        </div>
      ))}
      {entry.data?.platform_key?.available &&
        entry.data.requires_credential && (
          <Button
            variant="outline"
            disabled={pending}
            onClick={() => {
              void hosted
                .mutateAsync({ serviceSlug: slug, personal: true })
                .then((href) => window.location.assign(href))
                .catch(() => undefined);
            }}
          >
            {translateUi("Connect your own")}
          </Button>
        )}
      {!rows.length && (
        <Button
          variant="outline"
          disabled={
            pending ||
            optInBlocked ||
            entry.isLoading ||
            keys.isLoading ||
            Boolean(entry.error ?? keys.error)
          }
          onClick={() => {
            void activate().catch(() => undefined);
          }}
        >
          {translateUi("Connect")}
        </Button>
      )}
    </div>
  );
}
