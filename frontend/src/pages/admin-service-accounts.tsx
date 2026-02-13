import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
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
import { PageHeader } from "@/components/shared/page-header";
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
  Bot,
  Search,
  ChevronLeft,
  ChevronRight,
  Plus,
  Copy,
  AlertTriangle,
} from "lucide-react";
import { toast } from "sonner";
import { copyToClipboard } from "@/lib/utils";
import type { CreateServiceAccountResponse } from "@/types/service-accounts";

const PER_PAGE = 20;

// The create dialog is intentionally inlined here rather than extracted to a
// separate component, since it is only used in this page. Extract to
// components/dashboard/create-service-account-dialog.tsx if the file grows.
export function AdminServiceAccountsPage() {
  const navigate = useNavigate();
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

  const createForm = useForm<CreateServiceAccountFormData>({
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
      toast.success("Service account created");
    } catch (err) {
      if (err instanceof ApiError) {
        createForm.setError("root", { message: err.message });
      } else {
        toast.error("Failed to create service account");
      }
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Service Accounts"
        description="Manage machine-to-machine service accounts for programmatic access."
      />

      <div className="flex items-center justify-between gap-2">
        <form onSubmit={handleSearch} className="flex items-center gap-2">
          <div className="relative flex-1 max-w-sm">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search by name..."
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              className="pl-9"
            />
          </div>
          <Button type="submit" variant="outline" size="sm">
            Search
          </Button>
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
            >
              Clear
            </Button>
          )}
        </form>
        <Button size="sm" onClick={openCreateDialog}>
          <Plus className="mr-1 h-4 w-4" />
          Create Service Account
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`sa-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Bot className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load service accounts. Please try again.
          </p>
        </div>
      ) : accounts.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Bot className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            {search
              ? "No service accounts match your search."
              : "No service accounts found."}
          </p>
        </div>
      ) : (
        <>
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Client ID</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Scopes</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Last Used</TableHead>
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
                      <Badge
                        variant={sa.is_active ? "success" : "destructive"}
                      >
                        {sa.is_active ? "Active" : "Inactive"}
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
            <p className="text-sm text-muted-foreground">
              Showing {String((page - 1) * PER_PAGE + 1)}-
              {String(Math.min(page * PER_PAGE, total))} of {String(total)}{" "}
              service accounts
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
              >
                <ChevronLeft className="h-4 w-4" />
                Previous
              </Button>
              <span className="text-sm text-muted-foreground">
                Page {String(page)} of {String(totalPages)}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
              >
                Next
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </>
      )}

      {/* Create Service Account Dialog */}
      <Dialog open={createOpen} onOpenChange={closeCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Service Account</DialogTitle>
            <DialogDescription>
              Create a new machine-to-machine service account for programmatic
              API access.
            </DialogDescription>
          </DialogHeader>

          {createdResult ? (
            <div className="space-y-4">
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3">
                <div className="flex items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 text-amber-600" />
                  <p className="text-sm text-amber-700 dark:text-amber-400">
                    Save these credentials now. The client secret cannot be
                    retrieved later.
                  </p>
                </div>
              </div>

              <div className="space-y-3">
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Client ID
                  </p>
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
                          () => toast.success("Client ID copied"),
                          () => toast.error("Failed to copy"),
                        )
                      }
                    >
                      <Copy className="h-3 w-3" />
                    </Button>
                  </div>
                </div>

                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Client Secret
                  </p>
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
                          () => toast.success("Client secret copied"),
                          () => toast.error("Failed to copy"),
                        )
                      }
                    >
                      <Copy className="h-3 w-3" />
                    </Button>
                  </div>
                </div>
              </div>

              <DialogFooter>
                <Button onClick={closeCreateDialog}>Done</Button>
              </DialogFooter>
            </div>
          ) : (
            <Form {...createForm}>
              <form
                onSubmit={createForm.handleSubmit((data) =>
                  void handleCreate(data),
                )}
                className="space-y-4"
              >
                {createForm.formState.errors.root && (
                  <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                    {createForm.formState.errors.root.message}
                  </div>
                )}
                <FormField
                  control={createForm.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Name</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g. CI/CD Pipeline"
                          {...field}
                        />
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
                      <FormLabel>Description</FormLabel>
                      <FormControl>
                        <Input placeholder="Optional description" {...field} />
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
                      <FormLabel>Allowed Scopes</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g. openid proxy:* llm:proxy"
                          {...field}
                        />
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
                      <FormLabel>Role IDs (comma-separated, optional)</FormLabel>
                      <FormControl>
                        <Input placeholder="Optional" {...field} />
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
                      <FormLabel>Rate Limit Override (optional)</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          placeholder="Requests per second"
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
                  >
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={createMutation.isPending}>
                    Create
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
