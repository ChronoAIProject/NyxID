import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { ServiceAccountScopePicker } from "@/components/service-accounts/service-account-scope-picker";
import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useServiceAccounts,
  useCreateServiceAccount,
} from "@/hooks/use-service-accounts";
import {
  createServiceAccountSchema,
  type CreateServiceAccountFormData,
} from "@/schemas/service-accounts";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { canAdminWrite } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Search,
  ChevronLeft,
  ChevronRight,
  Copy,
  AlertTriangle,
} from "lucide-react";
import { RoboticArmIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { copyToClipboard } from "@/lib/utils";
import type { CreateServiceAccountResponse } from "@/types/service-accounts";

const PER_PAGE = 20;

// The create dialog is intentionally inlined here rather than extracted to a
// separate component, since it is only used in this page. Extract to
// components/dashboard/create-service-account-dialog.tsx if the file grows.
export function AdminServiceAccountsPage() {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [createdResult, setCreatedResult] =
    useState<CreateServiceAccountResponse | null>(null);

  const { data, isLoading, error } = useServiceAccounts(page, PER_PAGE, search);
  const createMutation = useCreateServiceAccount();

  const accounts = data?.service_accounts ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PER_PAGE));

  const createForm = useAppForm<CreateServiceAccountFormData>({
    resolver: zodResolver(createServiceAccountSchema),
    defaultValues: {
      name: "",
      description: "",
      allowed_scopes: "",
      role_ids: "",
      rate_limit_override: "",
    },
  });

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    setSearch(searchInput);
    setPage(1);
  }

  function openCreateDialog() {
    createForm.reset({
      name: "",
      description: "",
      allowed_scopes: "",
      role_ids: "",
      rate_limit_override: "",
    });
    setCreatedResult(null);
    setCreateOpen(true);
  }

  function closeCreateDialog() {
    setCreateOpen(false);
    setCreatedResult(null);
  }

  async function handleCreate(formData: CreateServiceAccountFormData) {
    try {
      const roleIds = formData.role_ids
        ? formData.role_ids
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean)
        : undefined;
      const rateLimit = formData.rate_limit_override
        ? Number(formData.rate_limit_override)
        : undefined;

      const result = await createMutation.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        allowed_scopes: formData.allowed_scopes,
        role_ids: roleIds,
        rate_limit_override: rateLimit,
      });
      setCreatedResult(result);
      toast.success(translateUi("Service account created"));
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to create service account"),
      );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("Service Accounts")}
        description={t("Manage machine-to-machine service accounts for programmatic access.")}
        actions={
          canWrite ? (
            <AddCtaButton label={t("Create Service Account")} onClick={openCreateDialog} />
          ) : null
        }
      />

      <form onSubmit={handleSearch} className="flex items-center gap-2">
        <div className="relative max-w-sm flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder={t("Search by name...")}
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
            className="pl-9"
          />
        </div>
        <Button type="submit" variant="outline" size="sm">{t("Search")}</Button>
        {search && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              setSearchInput("");
              setSearch("");
              setPage(1);
            }}
          >{t("Clear")}</Button>
        )}
      </form>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`sa-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <RoboticArmIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-12 font-medium text-muted-foreground">{t("Failed to load service accounts")}</p>
            <p className="text-xs text-muted-foreground">{t("Please try again later.")}</p>
          </div>
        </div>
      ) : accounts.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <RoboticArmIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-12 font-medium text-muted-foreground">{t("No service accounts found")}</p>
            <p className="text-xs text-muted-foreground">
              {search
                ? t("No service accounts match your search.")
                : t("There are no service accounts to display.")}
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* Mobile cards */}
          <div className="flex flex-col gap-3 md:hidden">
            {accounts.map((sa) => (
              <div
                key={sa.id}
                className="rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-overlay cursor-pointer"
                tabIndex={0}
                role="link"
                onClick={() =>
                  void navigate({
                    to: "/admin/service-accounts/$saId",
                    params: { saId: sa.id },
                  })
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    void navigate({
                      to: "/admin/service-accounts/$saId",
                      params: { saId: sa.id },
                    });
                  }
                }}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium truncate">{sa.name}</p>
                    <p className="text-xs text-muted-foreground font-mono truncate mt-0.5">
                      {sa.client_id}
                    </p>
                  </div>
                  <Badge variant={sa.is_active ? "success" : "destructive"}>
                    {sa.is_active ? t("Active") : t("Inactive")}
                  </Badge>
                </div>
                <div className="mt-3 flex items-center gap-3 text-xs text-muted-foreground">
                  <span>{t("Created ")}{formatDate(sa.created_at)}</span>
                </div>
              </div>
            ))}
          </div>

          {/* Desktop table */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("Name")}</TableHead>
                  <TableHead>{t("Client ID")}</TableHead>
                  <TableHead>{t("Status")}</TableHead>
                  <TableHead>{t("Scopes")}</TableHead>
                  <TableHead>{t("Created")}</TableHead>
                  <TableHead>{t("Last Used")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {accounts.map((sa) => (
                  <TableRow
                    key={sa.id}
                    className="cursor-pointer"
                    tabIndex={0}
                    role="link"
                    onClick={() =>
                      void navigate({
                        to: "/admin/service-accounts/$saId",
                        params: { saId: sa.id },
                      })
                    }
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        void navigate({
                          to: "/admin/service-accounts/$saId",
                          params: { saId: sa.id },
                        });
                      }
                    }}
                  >
                    <TableCell className="font-medium">{sa.name}</TableCell>
                    <TableCell className="font-mono text-xs">
                      {sa.client_id}
                    </TableCell>
                    <TableCell>
                      <Badge variant={sa.is_active ? "success" : "destructive"}>
                        {sa.is_active ? t("Active") : t("Inactive")}
                      </Badge>
                    </TableCell>
                    <TableCell
                      className="max-w-[200px] truncate text-xs"
                      title={sa.allowed_scopes}
                    >
                      {sa.allowed_scopes}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(sa.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(sa.last_authenticated_at)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          <div className="flex items-center justify-between">
            <p className="text-11 text-text-tertiary">{t("Showing ")}{String((page - 1) * PER_PAGE + 1)}-
              {String(Math.min(page * PER_PAGE, total))}{t(" of ")}{String(total)}{" "}{t("service accounts")}</p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="icon"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                aria-label={t("Previous page")}
              >
                <ChevronLeft className="h-3 w-3" />
              </Button>
              <span className="text-11 text-text-tertiary">{t("Page ")}{String(page)}{t(" of ")}{String(totalPages)}
              </span>
              <Button
                variant="outline"
                size="icon"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
                aria-label={t("Next page")}
              >
                <ChevronRight className="h-3 w-3" />
              </Button>
            </div>
          </div>
        </>
      )}

      {/* Create Service Account Dialog */}
      <Dialog open={createOpen} onOpenChange={closeCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Create Service Account")}</DialogTitle>
            <DialogDescription>{t("Create a new machine-to-machine service account for programmatic API access.")}</DialogDescription>
          </DialogHeader>

          {createdResult ? (
            <div className="space-y-4">
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3">
                <div className="flex items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 text-amber-600" />
                  <p className="text-sm text-amber-700 dark:text-amber-400">{t("Save these credentials now. The client secret cannot be retrieved later.")}</p>
                </div>
              </div>

              <div className="space-y-3">
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">{t("Client ID")}</p>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 rounded bg-muted px-2 py-1 text-sm font-mono">
                      {createdResult.client_id}
                    </code>
                    <Button
                      variant="outline"
                      size="icon"
                      className="h-8 w-8"
                      onClick={() =>
                        void copyToClipboard(createdResult.client_id).then(
                          () => toast.success(translateUi("Client ID copied")),
                          () => toast.error(translateUi("Failed to copy")),
                        )
                      }
                    >
                      <Copy className="h-3 w-3" />
                    </Button>
                  </div>
                </div>

                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">{t("Client Secret")}</p>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 rounded bg-muted px-2 py-1 text-sm font-mono break-all">
                      {createdResult.client_secret}
                    </code>
                    <Button
                      variant="outline"
                      size="icon"
                      className="h-8 w-8"
                      onClick={() =>
                        void copyToClipboard(createdResult.client_secret).then(
                          () => toast.success(translateUi("Client secret copied")),
                          () => toast.error(translateUi("Failed to copy")),
                        )
                      }
                    >
                      <Copy className="h-3 w-3" />
                    </Button>
                  </div>
                </div>
              </div>

              <DialogFooter>
                <Button onClick={closeCreateDialog}>{t("Done")}</Button>
              </DialogFooter>
            </div>
          ) : (
            <Form {...createForm}>
              <form
                onSubmit={createForm.handleSubmit(
                  (data) => void handleCreate(data),
                )}
                className="space-y-4"
              >
                <FormField
                  control={createForm.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Name")}</FormLabel>
                      <FormControl>
                        <Input placeholder={t("e.g. CI/CD Pipeline")} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="description"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Description")}</FormLabel>
                      <FormControl>
                        <Input placeholder={t("Optional description")} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="allowed_scopes"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Allowed Scopes")}</FormLabel>
                      <FormControl>
                        <ServiceAccountScopePicker ownerId={currentUser?.id ?? ""} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="role_ids"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Role IDs (comma-separated, optional)")}</FormLabel>
                      <FormControl>
                        <Input placeholder={t("Optional")} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="rate_limit_override"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Rate Limit Override (optional)")}</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          placeholder={t("Requests per second")}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={closeCreateDialog}
                  >{t("Cancel")}</Button>
                  <Button type="submit" variant="primary" isLoading={createMutation.isPending}>{t("Create")}</Button>
                </DialogFooter>
              </form>
            </Form>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
