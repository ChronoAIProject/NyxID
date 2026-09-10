import { useEffect, useState } from "react";
import QRCode from "qrcode";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  agentKeyErrorMessage,
  type AgentKeyPreview,
} from "@/schemas/agent-key-login";
import { formatAuthDeviceUserCodeInput } from "@/schemas/auth-device";

type Terminal = "approved" | "denied" | "expired";

export function PhoneApprovalPanel({
  code,
  interval,
  deadline,
  onTerminal,
  path,
  preview,
}: {
  code: string;
  interval: number;
  deadline: number;
  onTerminal: (state: Terminal) => void;
  path: "/login/device" | "/login/agent-key";
  preview: (code: string) => Promise<AgentKeyPreview>;
}) {
  const [qr, setQr] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    const url = new URL(path, window.location.origin);
    url.searchParams.set("user_code", code);
    void QRCode.toDataURL(url.toString(), {
      errorCorrectionLevel: "M",
      margin: 4,
      width: 208,
      color: { dark: "#0c0b14", light: "#e8e4f0" },
    })
      .then((image) => {
        if (!cancelled) setQr(image);
      })
      .catch(() => {
        if (!cancelled)
          setError("The QR code could not be displayed. Use the manual code.");
      });
    return () => {
      cancelled = true;
    };
  }, [code, path]);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let delay = Math.max(5, interval);
    const poll = async () => {
      if (cancelled) return;
      if (Date.now() >= deadline) {
        onTerminal("expired");
        return;
      }
      try {
        const response = await preview(code);
        if (cancelled) return;
        if (response.status === "approved" || response.status === "delivered") {
          onTerminal("approved");
          return;
        }
        if (response.status === "denied" || response.status === "expired") {
          onTerminal(response.status);
          return;
        }
        delay = Math.max(5, response.interval);
        setError(null);
      } catch (pollError) {
        if (cancelled) return;
        const errorCode = (pollError as { errorCode?: number }).errorCode;
        if (errorCode === 11900 || errorCode === 11901 || errorCode === 11907) {
          onTerminal("expired");
          return;
        }
        if (errorCode === 11904) {
          onTerminal("denied");
          return;
        }
        setError(agentKeyErrorMessage(pollError));
        delay = Math.min(30, delay + 5);
      }
      timer = setTimeout(() => void poll(), delay * 1000);
    };
    timer = setTimeout(() => void poll(), delay * 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [code, deadline, interval, onTerminal, preview]);
  return (
    <section className="flex flex-col items-center gap-3 border-t border-border/50 pt-4">
      <h2 className="text-[15px] font-semibold">Approve from your phone</h2>
      <div className="h-52 w-52">
        {qr && (
          <img
            src={qr}
            width={208}
            height={208}
            alt="Agent Key login QR code"
          />
        )}
      </div>
      <p className="font-mono text-[22px]">
        {formatAuthDeviceUserCodeInput(code)}
      </p>
      <p className="text-[12px] text-muted-foreground">
        Waiting for your phone's approval
      </p>
      {error && <ErrorBanner message={error} />}
    </section>
  );
}
