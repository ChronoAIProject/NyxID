import { Link } from "@tanstack/react-router";
import { CheckCircle2, Info } from "lucide-react";
import type { GlobalApprovalMode } from "@/types/approvals";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface SetupStep {
  readonly label: string;
  readonly description: string;
  readonly done: boolean;
}

export function ApprovalSetupWizard({
  hasChannel,
  channelEnabled,
  approvalEnabled,
  approvalMode,
  approvalSuspended,
  deliveryConfigured,
  webAvailable,
}: {
  readonly hasChannel: boolean;
  readonly channelEnabled: boolean;
  readonly approvalEnabled: boolean;
  readonly approvalMode?: GlobalApprovalMode;
  readonly approvalSuspended: boolean;
  readonly deliveryConfigured?: boolean;
  readonly webAvailable?: boolean;
}) {
  const steps: readonly SetupStep[] = [
    {
      label: webAvailable === true ? "Connect an optional notification channel" : "Connect a notification channel",
      description:
        "Connect Telegram or install the NyxID mobile app and sign in to register a device.",
      done: hasChannel,
    },
    {
      label: "Enable the channel",
      description:
        "Turn on Telegram or push notifications so approval requests can reach you.",
      done: channelEnabled,
    },
    {
      label: "Turn on approval protection",
      description:
        "Choose Writes only (default), All requests, or Off in approval settings. Auto-connected services remain available unless you add an explicit per-service policy.",
      done: approvalEnabled,
    },
  ];

  const allDone = approvalEnabled && !approvalSuspended && webAvailable === true;

  if (approvalEnabled && (approvalSuspended || webAvailable !== true)) {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-4">
        <Info className="mt-0.5 h-5 w-5 shrink-0 text-amber-500" />
        <div className="space-y-0.5">
          <p className="text-12 font-medium">{approvalSuspended ? "Server reports approval is suspended" : "Web-only approval support is unconfirmed"}</p>
          <p className="text-xs text-muted-foreground">
            Keep a configured notification channel enabled until the server confirms required approval and web decisions remain available without delivery.
          </p>
        </div>
      </div>
    );
  }

  if (allDone) {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-4">
        <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-500" />
        <div className="space-y-0.5">
          <p className="text-12 font-medium">
            Approval protection is active
          </p>
          <p className="text-xs text-muted-foreground">
            {deliveryConfigured === undefined ? "Notification configuration is unknown. " : deliveryConfigured ? "Notifications are configured; delivery can still fail. " : "Notifications are unavailable; use the web to decide. "}
            {approvalMode === "writes_only"
              ? "Writes and destructive operations require approval by default; reads and stateless model inference can proceed. "
              : "Programmatic proxy, LLM gateway, and SSH requests require your approval by default. "}
            Auto-connected services remain available
            unless you add an explicit per-service policy. You can approve via
            Telegram, the mobile app, or the{" "}
            <Link
              to="/approvals/history"
              className="underline underline-offset-2 hover:text-foreground"
            >
              Approval History
            </Link>{" "}
            page.
          </p>
        </div>
      </div>
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <span className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-hairline bg-overlay">
            <Info className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
          </span>
          Set up approval protection
        </CardTitle>
        <CardDescription>
          {webAvailable === true ? <>
          Approval protection can be enabled without notifications. Pending requests wait for a decision or expire without running. Review them in the{" "}
          <Link
            to="/approvals/history"
            className="underline underline-offset-2 hover:text-foreground"
          >
            Approval History
          </Link>{" "}
          page. Telegram and push are optional notifications.
          </> : <>
            Web-only approval support is unconfirmed. Connect and enable a notification channel before turning on approval protection on this server.
          </>}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ol className="space-y-4">
          {steps.map((step, idx) => (
            <li key={step.label} className="flex items-start gap-3">
              <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-hairline bg-overlay text-10 font-medium text-muted-foreground">
                {String(idx + 1)}
              </span>
              <div className="space-y-0.5">
                <p className="text-12 font-medium">
                  Step {String(idx + 1)}: {step.label}
                </p>
                <p className="text-xs text-muted-foreground">
                  {step.description}
                </p>
              </div>
            </li>
          ))}
        </ol>
      </CardContent>
    </Card>
  );
}
