import { useEffect, useRef, useState } from "react";
import { useParams, useNavigate } from "@tanstack/react-router";
import { Loader2, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ApiError } from "@/lib/api-client";
import { useRedeemInvite } from "@/hooks/use-orgs";

type RedeemState =
  | { readonly status: "pending" }
  | { readonly status: "error"; readonly message: string }
  | { readonly status: "success"; readonly orgId: string };

/**
 * Redemption page for org invite nonces. Automatically attempts the redeem
 * mutation once on mount, then redirects to the new org detail page on
 * success. Strict Mode double-invocation is guarded with a ref so the nonce
 * is only consumed once.
 */
export function OrgJoinPage() {
  const { nonce } = useParams({ strict: false }) as { nonce: string };
  const navigate = useNavigate();
  const redeemMutation = useRedeemInvite();
  const [state, setState] = useState<RedeemState>({ status: "pending" });
  const attemptedRef = useRef(false);

  useEffect(() => {
    if (attemptedRef.current) return;
    attemptedRef.current = true;

    void (async () => {
      try {
        const result = await redeemMutation.mutateAsync(nonce);
        setState({ status: "success", orgId: result.org_id });
        void navigate({ to: "/orgs/$orgId", params: { orgId: result.org_id } });
      } catch (err) {
        // Distinguish the dedicated `org_invite_expired` (410 / error_code
        // 8105) case from a generic invalid invite so the UI can point the
        // user at a concrete next step (issue #407). Pre-#407 this was
        // masked because the TTL index hard-deleted the row; the redeem
        // path then fell back to `org_invite_invalid`.
        let message =
          "Failed to redeem invite. The link may be invalid or expired.";
        if (err instanceof ApiError) {
          message =
            err.errorResponse.error === "org_invite_expired"
              ? "This invite has expired. Ask an admin to send a new invite."
              : err.message;
        }
        setState({ status: "error", message });
      }
    })();
    // nonce is a string from the URL and stable; the mutation is a hook ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce]);

  return (
    <div className="flex min-h-[50vh] items-center justify-center p-6">
      <Card className="w-full max-w-md">
        <CardContent className="flex flex-col items-center gap-4 py-12 text-center">
          {state.status === "pending" && (
            <>
              <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
              <p className="text-[12px] text-muted-foreground">
                Joining organization...
              </p>
            </>
          )}

          {state.status === "success" && (
            <>
              <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
              <p className="text-[12px] text-muted-foreground">
                Joined. Redirecting...
              </p>
            </>
          )}

          {state.status === "error" && (
            <>
              <XCircle className="h-8 w-8 text-destructive" />
              <div className="space-y-1">
                <p className="text-[12px] font-medium text-foreground">
                  Could not join organization
                </p>
                <p className="text-xs text-muted-foreground">{state.message}</p>
              </div>
              <Button
                variant="outline"
                onClick={() => void navigate({ to: "/orgs" })}
              >
                Back to organizations
              </Button>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
