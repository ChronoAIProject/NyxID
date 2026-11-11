import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { titleCase } from "@/schemas/platform-models";
import { PublicPreview } from "@/components/platform-models/public-preview";
import { useBreadcrumbLabel } from "@/components/layout/breadcrumb-context";
import { useState } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import {
  usePlatformModels,
  useAdminPlatformModel,
} from "@/hooks/use-platform-models";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import { adminModelSchema } from "@/schemas/platform-models";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ModelFormEditor } from "@/components/platform-models/model-form";
import { RoutesEditor } from "@/components/platform-models/routes-editor";
import { AccessEditor } from "@/components/platform-models/access-editor";
import { TariffsEditor } from "@/components/platform-models/tariffs-editor";
import { SaveError } from "@/components/platform-models/shared";

function useReadOnly() {
  const user = useAuthStore((s) => s.user);
  const enabled = useFeature(FEATURE_FLAG.INFERENCE_PLATFORM_MODELS);
  return !canAdminWrite(user) || !enabled;
}
export function AdminPlatformModelsPage() {
  useUiTranslation();
  const query = usePlatformModels(true);
  const readOnly = useReadOnly();
  const [creating, setCreating] = useState(false);
  const navigate = useNavigate();
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Platform Models")}
        description={translateUi("Public model contracts, private routes, access and retail price revisions.")}
      />
      {readOnly && (
        <p className="type-secondary text-muted-foreground">{translateUi("Read-only. Editing requires an administrator with Platform Models enabled.")}</p>
      )}
      {!readOnly && !creating && (
        <Button variant="primary" onClick={() => setCreating(true)}>{translateUi("Create model")}</Button>
      )}
      {creating && !readOnly && (
        <ModelFormEditor
          readOnly={false}
          onCancel={() => setCreating(false)}
          onSaved={(model) => {
            void navigate({
              to: "/admin/platform-models/$modelId",
              params: { modelId: model.id },
            });
          }}
        />
      )}
      <SaveError error={query.error} />
      {query.isLoading && <p className="type-body">{translateUi("Loading models…")}</p>}
      {query.data?.pages[0]?.models.length === 0 && (
        <p className="type-body text-muted-foreground">{translateUi("No models configured. Create a disabled draft to begin.")}</p>
      )}
      <div className="divide-y divide-border/30 rounded-xl border border-border/50">
        {query.data?.pages.flatMap((page) =>
          page.models.map((raw) => {
            const model = adminModelSchema.parse(raw);
            return (
              <Link
                key={model.id}
                to="/admin/platform-models/$modelId"
                params={{ modelId: model.id }}
                className="flex flex-wrap items-center justify-between gap-3 p-4 hover:bg-muted/30"
              >
                <div>
                  <h2 className="type-card-title">{model.display_name}</h2>
                  <p className="type-code text-muted-foreground">
                    {model.public_id}
                  </p>
                </div>
                <div className="type-secondary text-muted-foreground">
                  {titleCase(model.lifecycle)} ·{" "}
                  {model.enabled ? translateUi("Enabled") : translateUi("Disabled")}{translateUi(" · Revision")}{" "}
                  {model.revision}
                </div>
              </Link>
            );
          }),
        )}
      </div>
      {query.hasNextPage && (
        <Button
          variant="outline"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >{translateUi("Load more models")}</Button>
      )}
    </div>
  );
}
export function AdminPlatformModelDetailPage() {
  useUiTranslation();
  const { modelId } = useParams({
    from: "/dashboard/admin/platform-models/$modelId",
  });
  const query = useAdminPlatformModel(modelId);
  const readOnly = useReadOnly();
  const [epoch, setEpoch] = useState(0);
  const reload = () => {
    void query.refetch().then(() => setEpoch((n) => n + 1));
  };
  const model = query.data;
  useBreadcrumbLabel(model?.display_name);
  if (!model)
    return (
      <div className="space-y-4">
        <SaveError error={query.error} />
        {query.isLoading && <p className="type-body">{translateUi("Loading model…")}</p>}
      </div>
    );
  return (
    <div className="space-y-6">
      <PageHeader title={model.display_name} description={model.public_id} />
      {readOnly && (
        <p className="type-secondary text-muted-foreground">{translateUi("Read-only. Editing requires an administrator with Platform Models enabled.")}</p>
      )}
      <div className="rounded-xl border border-border/50 p-4 space-y-2">
        <p className="type-body">
          {model.execution_ready
            ? translateUi("Execution ready")
            : translateUi("Execution unavailable: {{value1}}", { value1: titleCase(model.blocking_reason) })}
        </p>
        <p className="type-secondary text-muted-foreground">{translateUi("Required fleet readiness: ")}{model.required_readiness.join(", ")}{translateUi(". Feature access does not override model readiness or entitlements. ")}</p>
      </div>
      <Tabs defaultValue="contract">
        <TabsList className="flex flex-wrap h-auto">
          <TabsTrigger value="contract">{translateUi("Public contract")}</TabsTrigger>
          <TabsTrigger value="routes">{translateUi("Private routes")}</TabsTrigger>
          <TabsTrigger value="preview">{translateUi("Public preview")}</TabsTrigger>
          <TabsTrigger value="access">{translateUi("Access")}</TabsTrigger>
          <TabsTrigger value="prices">{translateUi("Prices & readiness")}</TabsTrigger>
        </TabsList>
        <div key={`${model.id}-${model.revision}-${epoch}`}>
          <TabsContent value="contract">
            <ModelFormEditor
              model={model}
              readOnly={readOnly}
              reload={reload}
            />
          </TabsContent>
          <TabsContent value="preview">
            <PublicPreview model={model} />
          </TabsContent>
          <TabsContent value="routes">
            <RoutesEditor model={model} readOnly={readOnly} reload={reload} />
          </TabsContent>
          <TabsContent value="access">
            <AccessEditor model={model} readOnly={readOnly} reload={reload} />
          </TabsContent>
          <TabsContent value="prices">
            <TariffsEditor model={model} readOnly={readOnly} reload={reload} />
          </TabsContent>
        </div>
      </Tabs>
    </div>
  );
}
