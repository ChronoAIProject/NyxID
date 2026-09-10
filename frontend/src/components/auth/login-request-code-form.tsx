import { ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatAuthDeviceUserCodeInput } from "@/schemas/auth-device";

export function LoginRequestCodeForm({
  code,
  pending,
  previewPending,
  valid,
  onCodeChange,
  onContinue,
}: {
  code: string;
  pending: boolean;
  previewPending: boolean;
  valid: boolean;
  onCodeChange: (code: string) => void;
  onContinue: () => void;
}) {
  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        onContinue();
      }}
    >
      <label htmlFor="agent-key-code" className="text-[12px] font-medium">
        User code
      </label>
      <Input
        id="agent-key-code"
        data-sensitive
        autoComplete="off"
        value={code}
        maxLength={11}
        placeholder="ABCD-EFGH"
        disabled={pending}
        className="h-12 text-center font-mono text-[22px]"
        onChange={(event) =>
          onCodeChange(formatAuthDeviceUserCodeInput(event.target.value))
        }
      />
      <div className="flex justify-end">
        <Button
          variant="primary"
          type="submit"
          disabled={!valid || pending}
          isLoading={previewPending}
        >
          <ArrowRight className="size-3" />
          Continue
        </Button>
      </div>
    </form>
  );
}
