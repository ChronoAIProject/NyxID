import { useState, useEffect, useCallback } from "react"
import { useForm } from "react-hook-form"
import { zodResolver } from "@hookform/resolvers/zod"
import QRCode from "qrcode"
import { mfaVerifySchema, type MfaVerifyFormData } from "@/schemas/auth"
import { useMfaSetup } from "@/hooks/use-auth"
import { api } from "@/lib/api-client"
import { ApiError } from "@/lib/api-client"
import type { MfaSetupResponse } from "@/types/api"
import { useQueryClient } from "@tanstack/react-query"
import { copyToClipboard } from "@/lib/utils"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Copy, Check, ShieldCheck } from "lucide-react"
import { toast } from "sonner"

type MfaStep = "setup" | "verify" | "recovery"

interface MfaSetupDialogProps {
  readonly open: boolean
  readonly onOpenChange: (open: boolean) => void
}

export function MfaSetupDialog({ open, onOpenChange }: MfaSetupDialogProps) {
  const [step, setStep] = useState<MfaStep>("setup")
  const [setupData, setSetupData] = useState<MfaSetupResponse | null>(null)
  const [qrDataUrl, setQrDataUrl] = useState<string>("")
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const setupMutation = useMfaSetup()
  const queryClient = useQueryClient()

  const form = useForm<MfaVerifyFormData>({
    resolver: zodResolver(mfaVerifySchema),
    defaultValues: {
      code: "",
    },
  })

  useEffect(() => {
    if (copiedIndex === null) return
    const timer = setTimeout(() => setCopiedIndex(null), 2000)
    return () => clearTimeout(timer)
  }, [copiedIndex])

  async function handleSetup() {
    try {
      const data = await setupMutation.mutateAsync()
      setSetupData(data)
      const dataUrl = await QRCode.toDataURL(data.qr_code_url, {
        width: 200,
        margin: 2,
        color: {
          dark: "#fafafa",
          light: "#09090b",
        },
      })
      setQrDataUrl(dataUrl)
      setStep("verify")
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message)
      } else {
        toast.error("Failed to set up MFA")
      }
    }
  }

  async function onVerify(data: MfaVerifyFormData) {
    try {
      await api.post<void>("/auth/mfa/confirm", {
        code: data.code,
      })
      void queryClient.invalidateQueries({ queryKey: ["user"] })
      setStep("recovery")
      toast.success("MFA enabled successfully")
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message })
      } else {
        form.setError("root", {
          message: "Verification failed. Please try again.",
        })
      }
    }
  }

  const handleCopyCode = useCallback(async (code: string, index: number) => {
    try {
      await copyToClipboard(code)
      setCopiedIndex(index)
    } catch {
      toast.error("Failed to copy to clipboard")
    }
  }, [])

  function handleClose() {
    setStep("setup")
    setSetupData(null)
    setQrDataUrl("")
    form.reset()
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" aria-hidden="true" />
            {step === "setup" && "Enable Two-Factor Authentication"}
            {step === "verify" && "Scan QR Code"}
            {step === "recovery" && "Recovery Codes"}
          </DialogTitle>
          <DialogDescription>
            {step === "setup" &&
              "Add an extra layer of security to your account."}
            {step === "verify" &&
              "Scan this QR code with your authenticator app, then enter the verification code."}
            {step === "recovery" &&
              "Save these recovery codes in a safe place. You can use them to access your account if you lose your authenticator device."}
          </DialogDescription>
        </DialogHeader>

        {step === "setup" && (
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
              You will need an authenticator app like Google Authenticator,
              Authy, or 1Password to complete setup.
            </p>
            <Button
              onClick={handleSetup}
              className="w-full"
              isLoading={setupMutation.isPending}
            >
              Begin setup
            </Button>
          </div>
        )}

        {step === "verify" && (
          <div className="space-y-4">
            {qrDataUrl && (
              <div className="flex justify-center">
                <img
                  src={qrDataUrl}
                  alt="Scan this QR code with your authenticator app to set up two-factor authentication"
                  className="rounded-lg"
                  width={200}
                  height={200}
                />
              </div>
            )}

            {setupData?.secret && (
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground">
                  Or enter this code manually:
                </p>
                <code className="block rounded-md bg-muted p-2 text-center font-mono text-sm select-all">
                  {setupData.secret}
                </code>
              </div>
            )}

            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onVerify)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div
                    role="alert"
                    className="rounded-md bg-destructive/10 p-3 text-sm text-destructive"
                  >
                    {form.formState.errors.root.message}
                  </div>
                )}

                <FormField
                  control={form.control}
                  name="code"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Verification Code</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="000000"
                          maxLength={6}
                          inputMode="numeric"
                          autoComplete="one-time-code"
                          aria-label="Enter 6-digit verification code"
                          className="text-center font-mono text-lg tracking-widest"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <Button type="submit" className="w-full">
                  Verify and enable
                </Button>
              </form>
            </Form>
          </div>
        )}

        {step === "recovery" && setupData && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-2" role="list" aria-label="Recovery codes">
              {setupData.recovery_codes.map((code, index) => (
                <button
                  key={code}
                  type="button"
                  onClick={() => void handleCopyCode(code, index)}
                  className="flex items-center justify-between rounded-md bg-muted px-3 py-2 font-mono text-sm transition-colors hover:bg-muted/80"
                  aria-label={`Copy recovery code ${String(index + 1)}`}
                >
                  <span>{code}</span>
                  {copiedIndex === index ? (
                    <Check className="h-3 w-3 text-emerald-400" aria-hidden="true" />
                  ) : (
                    <Copy className="h-3 w-3 text-muted-foreground" aria-hidden="true" />
                  )}
                </button>
              ))}
            </div>

            <Button onClick={handleClose} className="w-full">
              Done
            </Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
