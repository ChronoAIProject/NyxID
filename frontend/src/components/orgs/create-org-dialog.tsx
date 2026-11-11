import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { ApiError } from "@/lib/api-client";
import { useCreateOrg } from "@/hooks/use-orgs";
import {
  createOrgRequestSchema,
  type CreateOrgRequest,
} from "@/schemas/orgs";

interface CreateOrgDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

export function CreateOrgDialog({ open, onOpenChange }: CreateOrgDialogProps) {
  useUiTranslation();
  const navigate = useNavigate();
  const createMutation = useCreateOrg();

  const form = useAppForm<CreateOrgRequest>({
    resolver: zodResolver(createOrgRequestSchema),
    defaultValues: {
      display_name: "",
      contact_email: "",
      avatar_url: "",
    },
  });

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
    if (!next) {
      form.reset();
    }
  }

  async function onSubmit(data: CreateOrgRequest) {
    try {
      const org = await createMutation.mutateAsync(data);
      toast.success(translateUi(`Organization "${org.display_name ?? "Untitled"}" created`));
      handleOpenChange(false);
      void navigate({ to: "/orgs/$orgId", params: { orgId: org.id } });
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error(translateErrorMessage("Failed to create organization"));
      }
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{translateUi("Create Organization")}</DialogTitle>
          <DialogDescription>{translateUi("Organizations let you share services and credentials with teammates. You become the first admin.")}</DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form
            onSubmit={(e) => void form.handleSubmit(onSubmit)(e)}
            className="space-y-4"
          >
            <FormField
              control={form.control}
              name="display_name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Display name")}</FormLabel>
                  <FormControl>
                    <Input placeholder={translateUi("Acme Inc.")} autoFocus {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="contact_email"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Contact email (optional)")}</FormLabel>
                  <FormControl>
                    <Input
                      type="email"
                      placeholder={translateUi("team@example.com")}
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="avatar_url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Avatar URL (optional)")}</FormLabel>
                  <FormControl>
                    <Input placeholder="https://..." {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {form.formState.errors.root && (
              <p className="text-14 text-destructive">
                {form.formState.errors.root.message}
              </p>
            )}

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => handleOpenChange(false)}
                disabled={createMutation.isPending}
              >{translateUi("Cancel")}</Button>
              <Button
                variant="primary"
                type="submit"
                isLoading={createMutation.isPending}
                disabled={!form.watch("display_name").trim()}
              >{translateUi("Create Organization")}</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
