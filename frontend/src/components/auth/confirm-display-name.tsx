import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { useUpdateDisplayName } from "@/hooks/use-update-display-name";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { confirmDisplayNameSchema } from "@/schemas/auth";

interface ConfirmDisplayNameProps {
  readonly userId: string;
  readonly initialName: string;
  readonly onContinue: () => void;
}

export function ConfirmDisplayName({
  userId,
  initialName,
  onContinue,
}: ConfirmDisplayNameProps) {
  const { t } = useUiTranslation();
  const updateName = useUpdateDisplayName(userId);
  const form = useAppForm({
    resolver: zodResolver(confirmDisplayNameSchema),
    defaultValues: { display_name: initialName },
  });
  const saving = form.formState.isSubmitting;

  const save = form.handleSubmit(async (values) => {
    try {
      await updateName.mutateAsync(values);
    } catch {
      form.setError("root", { type: "server" });
      return;
    }
    onContinue();
  });

  return (
    <div>
      <h1 className="type-page-title">{t("Choose your NyxID display name")}</h1>
      <p className="mt-1 text-15 text-muted-foreground">
        {t("You can change this later in Settings.")}
      </p>
      <Form {...form}>
        <form className="mt-8 space-y-5" onSubmit={save}>
          <FormField
            control={form.control}
            name="display_name"
            render={({ field }) => (
              <FormItem>
                <FormLabel>{t("Display name")}</FormLabel>
                <FormControl>
                  <Input {...field} autoComplete="nickname" readOnly={saving} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          {form.formState.errors.root && (
            <p role="alert" className="type-validation-error text-destructive">
              {t("Failed to update profile")}
            </p>
          )}
          <div className="grid gap-2">
            <Button type="submit" variant="primary" isLoading={saving}>
              {t("Save and continue")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              disabled={saving}
              onClick={onContinue}
            >
              {t("Skip")}
            </Button>
          </div>
        </form>
      </Form>
    </div>
  );
}
