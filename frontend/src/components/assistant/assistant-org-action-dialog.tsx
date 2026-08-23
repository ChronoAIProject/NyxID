import { useRef, useState } from "react";
import { Building2, ShieldAlert } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import {
  assertSecretFreeReadBack,
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
  isNotFound,
  SECRET_VALUE_PATTERN,
} from "./assistant-action-dialog-utils";

const actions = [
  "create",
  "update",
  "delete",
  "member_add",
  "member_remove",
  "member_update_role",
  "invite",
  "set_primary",
] as const;
export type AssistantOrgAction = (typeof actions)[number];

const orgEvidenceSchema = z
  .object({
    id: z.string().min(1),
    your_role: z.enum(["admin", "member", "viewer"]),
    member_count: z.number().int().nonnegative(),
    is_primary: z.boolean(),
    active_invite_count: z.number().int().nonnegative(),
    remote_credential_integrity_verification_opt_out: z.boolean(),
    created_at: z.string(),
    updated_at: z.string(),
  })
  .strict();

const memberEvidenceSchema = z
  .object({
    membership_id: z.string().min(1),
    user_id: z.string().min(1),
    role: z.enum(["admin", "member", "viewer"]),
    scope_source: z.enum(["inherit", "override"]),
    allowed_service_ids: z.array(z.string()).nullable(),
    effective_allowed_service_ids: z.array(z.string()).nullable(),
    created_at: z.string(),
    revoked_at: z.string().nullable(),
  })
  .strict();

const responseSchema = z
  .object({
    resource: z.object({ orgId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
  })
  .strict();

function actionTitle(action: AssistantOrgAction): string {
  return {
    create: "Create organization",
    update: "Update organization",
    delete: "Delete organization",
    member_add: "Add organization member",
    member_remove: "Remove organization member",
    member_update_role: "Change member role",
    invite: "Create organization invite",
    set_primary: "Set primary organization",
  }[action];
}

function textParam(params: Record<string, unknown>, key: string): string {
  const value = params[key];
  return typeof value === "string" ? value : "";
}

export interface AssistantOrgActionDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly action: AssistantOrgAction;
  readonly params: Record<string, unknown>;
  readonly onComplete: (orgId: string) => void;
}

export function AssistantOrgActionDialog({
  open,
  onOpenChange,
  actionRequestId,
  action,
  params,
  onComplete,
}: AssistantOrgActionDialogProps) {
  const [displayName, setDisplayName] = useState(textParam(params, "displayName"));
  const [slug, setSlug] = useState(textParam(params, "slug"));
  const [contactEmail, setContactEmail] = useState(textParam(params, "contactEmail"));
  const [avatarUrl, setAvatarUrl] = useState(textParam(params, "avatarUrl"));
  const [memberId, setMemberId] = useState(textParam(params, "memberId") || textParam(params, "userId"));
  const [userId, setUserId] = useState(textParam(params, "userId"));
  const [role, setRole] = useState(textParam(params, "role") || "member");
  const [ttlHours, setTtlHours] = useState(String(params.ttlHours ?? "24"));
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const destructive = action === "delete" || action === "member_remove";
  const orgId = textParam(params, "orgId");

  function close() {
    pendingRef.current = false;
    setPending(false);
    setError(null);
    setConfirmed(false);
    setResultId(null);
    onOpenChange(false);
  }

  async function readOrg(id: string) {
    const raw = await api.get<unknown>(`/orgs/${encodeURIComponent(id)}/authorization`);
    assertSecretFreeReadBack(raw);
    return orgEvidenceSchema.parse(raw);
  }

  async function readMember(org: string, member: string) {
    const raw = await api.get<unknown>(
      `/orgs/${encodeURIComponent(org)}/members/${encodeURIComponent(member)}/authorization`,
    );
    assertSecretFreeReadBack(raw);
    return memberEvidenceSchema.parse(raw);
  }

  async function submit() {
    if (pendingRef.current || resultId) return;
    if (destructive && !confirmed) {
      setError("Confirm this destructive change to continue.");
      return;
    }
    if ((action === "create" || action === "update") && SECRET_VALUE_PATTERN.test(displayName)) {
      setError("Organization names cannot contain secret-shaped values.");
      return;
    }
    if (action === "create" && !displayName.trim()) {
      setError("Enter an organization name.");
      return;
    }
    if (action !== "create" && !orgId) {
      setError("The organization reference is missing.");
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const before = action === "create" ? null : await readOrg(orgId);
      const beforeMember =
        (action === "member_remove" || action === "member_update_role") && memberId
          ? await readMember(orgId, memberId)
          : null;
      if (action === "member_update_role" && beforeMember?.role === role && !beforeMember.revoked_at) {
        throw new Error("That member already has the requested role.");
      }
      if ((action === "member_add" || action === "member_remove" || action === "member_update_role") && !memberId && action !== "member_add") {
        throw new Error("The member reference is missing.");
      }
      if (action === "member_add" && !userId) {
        throw new Error("The user reference is missing.");
      }
      const payload: Record<string, unknown> = { ...params, actionRequestId };
      if (destructive) payload.confirmed = confirmed;
      if (action === "create" || action === "update") {
        payload.displayName = displayName.trim() || undefined;
        payload.slug = slug.trim() || undefined;
        payload.contactEmail = contactEmail.trim() || undefined;
        payload.avatarUrl = avatarUrl.trim() || undefined;
      }
      if (action === "member_add") {
        payload.userId = userId;
        payload.role = role;
      }
      if (action === "member_remove" || action === "member_update_role") {
        payload.memberId = memberId;
      }
      if (action === "member_update_role") {
        payload.role = role;
        payload.expectedRole = beforeMember?.role;
      }
      if (action === "invite") {
        payload.role = role;
        payload.ttlHours = Number(ttlHours);
      }
      const raw = await api.post<unknown>(
        `/assistant/actions/org/org/${action.replaceAll("_", "-")}`,
        payload,
      );
      const response = responseSchema.parse(raw);
      const id = response.resource.orgId;
      if (action === "delete") {
        try {
          await readOrg(id);
          throw new Error("The organization still has a live authorization projection.");
        } catch (caught) {
          if (!isNotFound(caught)) throw caught;
        }
      } else {
        const after = await readOrg(id);
        if (after.id !== id) throw new Error("NyxID returned a different organization identity.");
        if (action === "create" && after.member_count < 1) {
          throw new Error("NyxID did not show the created organization membership.");
        }
        if (action === "set_primary" && !after.is_primary) {
          throw new Error("NyxID did not mark this organization as primary.");
        }
        if (action === "member_add" && !response.replayed && before && after.member_count <= before.member_count) {
          throw new Error("NyxID did not show the new organization member.");
        }
        if (action === "member_remove") {
          if (!before || (!response.replayed && after.member_count >= before.member_count)) {
            throw new Error("NyxID did not show the member removal.");
          }
          const removed = await readMember(id, memberId);
          if (removed.revoked_at === null) throw new Error("NyxID did not show the member as revoked.");
        }
        if (action === "member_update_role") {
          const changed = await readMember(id, memberId);
          if (changed.role !== role || changed.revoked_at !== null) {
            throw new Error("NyxID did not show the requested member role.");
          }
        }
        if (action === "invite" && !response.replayed && before && after.active_invite_count <= before.active_invite_count) {
          throw new Error("NyxID did not show the new organization invite.");
        }
        if ((action === "update" || action === "set_primary") && !response.replayed && before && action === "update" && !isNewerTimestamp(before.updated_at, after.updated_at)) {
          throw new Error("NyxID did not show a newer organization state.");
        }
      }
      setResultId(id);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not complete this organization action."));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Building2 className="size-4" />
            {resultId ? "Organization change confirmed" : actionTitle(action)}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "The canonical organization projection confirms the requested state."
              : destructive
                ? "This access change is destructive and must be confirmed every time."
                : "NyxID applies this change in your signed-in browser session."}
          </DialogDescription>
        </DialogHeader>
        {!resultId ? (
          <div className="space-y-4 border-y border-border py-4">
            {(action === "create" || action === "update") && (
              <>
                <div className="space-y-2"><Label htmlFor="org-name">Organization name</Label><Input id="org-name" value={displayName} onChange={(event) => setDisplayName(event.target.value)} autoComplete="off" /></div>
                <div className="space-y-2"><Label htmlFor="org-slug">Slug</Label><Input id="org-slug" value={slug} onChange={(event) => setSlug(event.target.value)} autoComplete="off" /></div>
                <div className="space-y-2"><Label htmlFor="org-contact">Contact email</Label><Input id="org-contact" value={contactEmail} onChange={(event) => setContactEmail(event.target.value)} autoComplete="email" /></div>
                <div className="space-y-2"><Label htmlFor="org-avatar">Avatar URL</Label><Input id="org-avatar" value={avatarUrl} onChange={(event) => setAvatarUrl(event.target.value)} autoComplete="url" /></div>
              </>
            )}
            {action === "member_add" && <><div className="space-y-2"><Label htmlFor="org-user-id">User ID</Label><Input id="org-user-id" value={userId} onChange={(event) => setUserId(event.target.value)} /></div><div className="space-y-2"><Label htmlFor="org-member-role">Role</Label><Input id="org-member-role" value={role} onChange={(event) => setRole(event.target.value)} /></div></>}
            {(action === "member_remove" || action === "member_update_role") && <div className="space-y-2"><Label htmlFor="org-member-id">Member ID</Label><Input id="org-member-id" value={memberId} onChange={(event) => setMemberId(event.target.value)} /></div>}
            {action === "member_update_role" && <div className="space-y-2"><Label htmlFor="org-role">Role</Label><Input id="org-role" value={role} onChange={(event) => setRole(event.target.value)} /></div>}
            {action === "invite" && <><div className="space-y-2"><Label htmlFor="invite-role">Invite role</Label><Input id="invite-role" value={role} onChange={(event) => setRole(event.target.value)} /></div><div className="space-y-2"><Label htmlFor="invite-ttl">Invite lifetime (hours)</Label><Input id="invite-ttl" type="number" min={1} max={720} value={ttlHours} onChange={(event) => setTtlHours(event.target.value)} /></div></>}
            {action !== "create" && action !== "update" && action !== "member_add" && action !== "member_remove" && action !== "member_update_role" && action !== "invite" ? <p className="font-mono text-xs text-muted-foreground">{orgId}</p> : null}
            {destructive && <label className="flex items-start gap-2 text-xs"><Checkbox checked={confirmed} onCheckedChange={(value) => setConfirmed(value === true)} /><span className="flex items-center gap-1"><ShieldAlert className="size-3" />I understand this access change is destructive.</span></label>}
          </div>
        ) : null}
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>
          {resultId ? <Button type="button" onClick={() => { onComplete(resultId); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant={destructive ? "destructive" : "primary"} isLoading={pending} disabled={pending || (destructive && !confirmed)} onClick={() => void submit()}>{destructive ? "Confirm change" : "Continue"}</Button></>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
