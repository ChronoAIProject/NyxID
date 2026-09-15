import { useEffect } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useUpdateDeveloperApp } from "@/hooks/use-developer-apps";
import { useUploadAppLogo } from "@/hooks/use-oauth-branding";
import type { OAuthClient } from "@/types/api";
import {
  homepageFormSchema,
  type HomepageForm,
} from "@/schemas/oauth-branding";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
} from "@/components/ui/card";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";

export function BrandingCard({ app }: { readonly app: OAuthClient }) {
  const upload = useUploadAppLogo(app.id);
  const update = useUpdateDeveloperApp();
  const form = useAppForm<HomepageForm>({
    resolver: zodResolver(homepageFormSchema),
    defaultValues: { homepage_url: app.homepage_url ?? "" },
  });
  const { reset } = form;
  useEffect(() => {
    reset({ homepage_url: app.homepage_url ?? "" });
  }, [app.homepage_url, reset]);
  async function save(data: HomepageForm) {
    try {
      const updated = await update.mutateAsync({ clientId: app.id, data });
      reset({ homepage_url: updated.homepage_url ?? "" });
    } catch {
      /* Rendered below. */
    }
  }
  const verified =
    app.branding_verified_revision != null &&
    app.branding_verified_revision === app.branding_revision;
  return (
    <Card>
      <CardHeader>
        <CardTitle>White labeling</CardTitle>
        <CardDescription>
          Your app identity appears on the sign-in handoff and connection
          checklist.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-3">
          {app.logo_url && (
            <img
              src={app.logo_url}
              alt={`${app.client_name} logo`}
              className="size-12 rounded-lg object-contain"
            />
          )}
          <span className="text-sm font-medium">{app.client_name}</span>
          {verified && <Badge variant="secondary">Verified</Badge>}
        </div>
        {upload.error && <ErrorBanner message={upload.error.message} />}
        <div className="space-y-2">
          <label htmlFor={`logo-${app.id}`} className="text-xs font-medium">
            App logo
          </label>
          <Input
            id={`logo-${app.id}`}
            type="file"
            accept="image/png,image/webp"
            disabled={upload.isPending}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) upload.mutate(file);
              event.target.value = "";
            }}
          />
          <p className="text-xs text-muted-foreground">
            PNG or WebP, up to 256 KiB and 512 × 512 pixels. Branding edits
            clear the Verified mark until an admin reviews them.
          </p>
        </div>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="space-y-4">
            {update.error && <ErrorBanner message={update.error.message} />}
            <FormField
              control={form.control}
              name="homepage_url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Homepage</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      placeholder="https://your-app.example"
                      maxLength={2048}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="ghost"
                disabled={!form.formState.isDirty || update.isPending}
                onClick={() => form.reset()}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={!form.formState.isDirty || update.isPending}
                isLoading={update.isPending}
              >
                Save homepage
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}
