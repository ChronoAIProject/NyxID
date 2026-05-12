import { Link } from "@tanstack/react-router";
import { CheckCircle2, Circle, Info } from "lucide-react";
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
}: {
  readonly hasChannel: boolean;
  readonly channelEnabled: boolean;
  readonly approvalEnabled: boolean;
}) {
  const steps: readonly SetupStep[] = [
    {
      label: "Connect a notification channel",
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
        "Enable the global approval toggle. Programmatic proxy, LLM gateway, and SSH requests will require your approval.",
      done: approvalEnabled,
    },
  ];

  const allDone = steps.every((s) => s.done);

  if (allDone) {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-4">
        <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-500" />
        <div className="space-y-0.5">
          <p className="text-[12px] font-medium">
            Approval protection is active
          </p>
          <p className="text-xs text-muted-foreground">
            Programmatic proxy, LLM gateway, and SSH requests require your
            approval. You can approve via Telegram, the mobile app, or the{" "}
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
          <span className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-white/[0.08] bg-white/[0.04]">
            <Info className="h-4 w-4 text-blue-500" aria-hidden="true" />
          </span>
          Set up approval protection
        </CardTitle>
        <CardDescription>
          Approval protection requires a notification channel so you can
          approve or reject requests in real time. Follow these steps to get
          started. You can always approve requests from the{" "}
          <Link
            to="/approvals/history"
            className="underline underline-offset-2 hover:text-foreground"
          >
            Approval History
          </Link>{" "}
          page as a fallback.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ol className="space-y-4">
          {steps.map((step, idx) => (
            <li key={step.label} className="flex items-start gap-3">
              <span className="mt-0.5 shrink-0">
                {step.done ? (
                  <CheckCircle2 className="h-5 w-5 text-emerald-500" />
                ) : (
                  <Circle className="h-5 w-5 text-muted-foreground/40" />
                )}
              </span>
              <div className="space-y-0.5">
                <p
                  className={
                    step.done
                      ? "text-[12px] text-muted-foreground line-through"
                      : "text-[12px] font-medium"
                  }
                >
                  Step {String(idx + 1)}: {step.label}
                </p>
                {!step.done && (
                  <p className="text-xs text-muted-foreground">
                    {step.description}
                  </p>
                )}
              </div>
            </li>
          ))}
        </ol>
      </CardContent>
    </Card>
  );
}
