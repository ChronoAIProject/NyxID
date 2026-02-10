import { useState, useEffect, useRef, useCallback } from "react";
import type { ProviderConfig, DeviceCodePollResponse } from "@/types/api";
import {
  useInitiateDeviceCode,
  usePollDeviceCode,
} from "@/hooks/use-providers";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  ExternalLink,
  CheckCircle2,
  AlertCircle,
  Copy,
  Loader2,
} from "lucide-react";
import { ApiError } from "@/lib/api-client";
import { toast } from "sonner";

type FlowStep = "requesting" | "show_code" | "success" | "error";

interface DeviceCodeDialogProps {
  readonly provider: ProviderConfig;
  readonly onClose: () => void;
}

export function DeviceCodeDialog({
  provider,
  onClose,
}: DeviceCodeDialogProps) {
  const [step, setStep] = useState<FlowStep>("requesting");
  const [userCode, setUserCode] = useState("");
  const [verificationUri, setVerificationUri] = useState("");
  const [, setStateToken] = useState("");
  const [, setPollInterval] = useState(5);
  const [, setExpiresIn] = useState(0);
  const [secondsRemaining, setSecondsRemaining] = useState(0);
  const [errorMessage, setErrorMessage] = useState("");

  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const countdownTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isMountedRef = useRef(true);

  const initiateMutation = useInitiateDeviceCode();
  const pollMutation = usePollDeviceCode();

  // Cleanup on unmount
  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      if (pollTimerRef.current) {
        clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      if (countdownTimerRef.current) {
        clearInterval(countdownTimerRef.current);
        countdownTimerRef.current = null;
      }
    };
  }, []);

  // Countdown timer
  useEffect(() => {
    if (step !== "show_code" || secondsRemaining <= 0) return;

    countdownTimerRef.current = setInterval(() => {
      if (!isMountedRef.current) return;
      setSecondsRemaining((prev) => {
        if (prev <= 1) {
          if (countdownTimerRef.current) {
            clearInterval(countdownTimerRef.current);
            countdownTimerRef.current = null;
          }
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => {
      if (countdownTimerRef.current) {
        clearInterval(countdownTimerRef.current);
        countdownTimerRef.current = null;
      }
    };
  }, [step, secondsRemaining]);

  const schedulePoll = useCallback(
    (state: string, interval: number) => {
      if (!isMountedRef.current) return;

      pollTimerRef.current = setTimeout(() => {
        if (!isMountedRef.current) return;

        pollMutation.mutate(
          { providerId: provider.id, state },
          {
            onSuccess: (data: DeviceCodePollResponse) => {
              if (!isMountedRef.current) return;

              switch (data.status) {
                case "pending":
                  schedulePoll(state, data.interval ?? interval);
                  break;
                case "slow_down":
                  schedulePoll(state, data.interval ?? interval + 5);
                  if (data.interval) {
                    setPollInterval(data.interval);
                  }
                  break;
                case "complete":
                  setStep("success");
                  break;
                case "expired":
                  setErrorMessage(
                    "Authentication expired. Please try again.",
                  );
                  setStep("error");
                  break;
                case "denied":
                  setErrorMessage("Authentication was denied.");
                  setStep("error");
                  break;
              }
            },
            onError: () => {
              // Transient network error - keep polling
              if (isMountedRef.current) {
                schedulePoll(state, interval);
              }
            },
          },
        );
      }, interval * 1000);
    },
    [pollMutation, provider.id],
  );

  // Auto-initiate on mount
  useEffect(() => {
    void handleInitiate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleInitiate() {
    setErrorMessage("");
    setStep("requesting");
    try {
      const response = await initiateMutation.mutateAsync(provider.id);
      if (!isMountedRef.current) return;

      setUserCode(response.user_code);
      setVerificationUri(response.verification_uri);
      setStateToken(response.state);
      setPollInterval(response.interval);
      setExpiresIn(response.expires_in);
      setSecondsRemaining(response.expires_in);
      setStep("show_code");

      // Start polling automatically
      schedulePoll(response.state, response.interval);
    } catch (error) {
      if (!isMountedRef.current) return;
      if (error instanceof ApiError) {
        setErrorMessage(error.message);
      } else {
        setErrorMessage("Failed to request device code");
      }
      setStep("error");
    }
  }

  function handleCopyCode() {
    void navigator.clipboard.writeText(userCode).then(() => {
      toast.success("Code copied to clipboard");
    });
  }

  function handleRetry() {
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
    if (countdownTimerRef.current) {
      clearInterval(countdownTimerRef.current);
      countdownTimerRef.current = null;
    }
    setUserCode("");
    setVerificationUri("");
    setStateToken("");
    setErrorMessage("");
    setSecondsRemaining(0);
    void handleInitiate();
  }

  function handleClose() {
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
    if (countdownTimerRef.current) {
      clearInterval(countdownTimerRef.current);
      countdownTimerRef.current = null;
    }
    onClose();
  }

  function formatTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${String(mins)}:${String(secs).padStart(2, "0")}`;
  }

  return (
    <Dialog open onOpenChange={handleClose}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Connect to {provider.name}</DialogTitle>
          <DialogDescription>
            {step === "requesting" && "Requesting authentication code..."}
            {step === "show_code" &&
              "Enter the code below on the authentication page to connect your account."}
            {step === "success" &&
              `Successfully connected to ${provider.name}.`}
            {step === "error" && "An error occurred during the connection."}
          </DialogDescription>
        </DialogHeader>

        {step === "requesting" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <p className="text-sm text-muted-foreground">
              Requesting code from {provider.name}...
            </p>
          </div>
        )}

        {step === "show_code" && (
          <div className="space-y-5">
            {/* User code display */}
            <div className="flex flex-col items-center gap-3 rounded-lg border-2 border-dashed border-primary/30 bg-primary/5 p-6">
              <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Your code
              </p>
              <div className="flex items-center gap-3">
                <code className="text-3xl font-bold tracking-[0.3em] font-mono text-primary">
                  {userCode}
                </code>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={handleCopyCode}
                  className="h-8 w-8 p-0"
                  title="Copy code"
                >
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
            </div>

            {/* Verification link */}
            <div className="flex justify-center">
              <Button
                type="button"
                variant="default"
                size="lg"
                asChild
              >
                <a
                  href={verificationUri}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  <ExternalLink className="mr-2 h-4 w-4" />
                  Open {provider.name} Authentication
                </a>
              </Button>
            </div>

            {/* Instructions */}
            <div className="rounded-md bg-muted p-3 text-sm text-muted-foreground">
              <ol className="list-decimal list-inside space-y-1">
                <li>Click the link above to open the authentication page</li>
                <li>Enter the code shown above</li>
                <li>Sign in with your account</li>
              </ol>
            </div>

            {/* Polling status */}
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <div className="flex items-center gap-2">
                <Loader2 className="h-3 w-3 animate-spin" />
                <span>Waiting for authentication...</span>
              </div>
              {secondsRemaining > 0 && (
                <span>Expires in {formatTime(secondsRemaining)}</span>
              )}
            </div>

            <DialogFooter>
              <Button type="button" variant="outline" onClick={handleClose}>
                Cancel
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === "success" && (
          <div className="space-y-4">
            <div className="flex flex-col items-center gap-3 py-4">
              <CheckCircle2 className="h-10 w-10 text-emerald-500" />
              <p className="text-sm text-muted-foreground text-center">
                Your {provider.name} account has been connected successfully.
                Tokens are encrypted and stored securely.
              </p>
            </div>
            <DialogFooter>
              <Button type="button" onClick={handleClose}>
                Done
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === "error" && (
          <div className="space-y-4">
            <div className="flex flex-col items-center gap-3 py-4">
              <AlertCircle className="h-10 w-10 text-destructive" />
              <p className="text-sm text-destructive text-center">
                {errorMessage}
              </p>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={handleClose}>
                Cancel
              </Button>
              <Button type="button" onClick={handleRetry}>
                Try Again
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
