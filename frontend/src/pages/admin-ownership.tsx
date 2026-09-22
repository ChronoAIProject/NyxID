import { useState } from "react";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import { useOwnershipResources } from "@/hooks/use-ownership-transfers";
import type {
  OwnershipResource,
  OwnershipResourceKind,
} from "@/types/ownership-transfers";
import { OwnershipTransferDialog } from "@/components/shared/ownership-transfer-dialog";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function AdminOwnershipPage() {
  const user = useAuthStore((state) => state.user);
  const allowed = canAdminWrite(user);
  const [kind, setKind] = useState<OwnershipResourceKind>("service");
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<OwnershipResource | null>(null);
  const resources = useOwnershipResources(kind, search, offset, allowed);
  if (!allowed)
    return (
      <p>
        NyxID admin access is required to browse all assets. Owners can transfer
        their assets from the asset settings.
      </p>
    );
  return (
    <div className="space-y-6">
      <PageHeader
        title="Ownership transfers"
        description="Reassign catalog services and channel bots between people and organizations."
      />
      <div className="flex flex-wrap gap-3">
        <Select
          value={kind}
          onValueChange={(value) => {
            setKind(value as OwnershipResourceKind);
            setOffset(0);
            setSelected(null);
          }}
        >
          <SelectTrigger aria-label="Resource type" className="w-52">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="service">Catalog services</SelectItem>
            <SelectItem value="channel_bot">Channel bots</SelectItem>
          </SelectContent>
        </Select>
        <Input
          aria-label="Search resources"
          className="max-w-sm"
          placeholder="Search name, slug or ID"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setOffset(0);
          }}
        />
      </div>
      {resources.error && (
        <p role="alert" className="text-destructive">
          {resources.error.message}
        </p>
      )}
      {resources.isLoading ? (
        <p>Loading resources…</p>
      ) : (
        <div className="overflow-x-auto rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Resource</TableHead>
                <TableHead>Current owner</TableHead>
                <TableHead className="text-right">Action</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {resources.data?.items.map((resource) => (
                <TableRow key={resource.id}>
                  <TableCell>
                    <p className="font-medium">{resource.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {resource.slug || resource.platform}
                    </p>
                  </TableCell>
                  <TableCell className="break-all font-mono text-xs">
                    {resource.owner_user_id}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="outline"
                      onClick={() => setSelected(resource)}
                    >
                      Transfer ownership
                      <span className="sr-only"> of {resource.name}</span>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
              {!resources.error && resources.data?.items.length === 0 && (
                <TableRow>
                  <TableCell
                    colSpan={3}
                    className="py-8 text-center text-muted-foreground"
                  >
                    No matching resources.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      )}
      <div className="flex justify-between">
        <Button
          variant="outline"
          disabled={offset === 0 || resources.isFetching}
          onClick={() => setOffset((current) => Math.max(0, current - 50))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          disabled={resources.data?.next_offset == null || resources.isFetching}
          onClick={() => setOffset(resources.data?.next_offset ?? offset)}
        >
          Next
        </Button>
      </div>
      {selected && (
        <OwnershipTransferDialog
          key={`${kind}:${selected.id}`}
          kind={kind}
          resource={selected}
          onClose={() => setSelected(null)}
        />
      )}
    </div>
  );
}
