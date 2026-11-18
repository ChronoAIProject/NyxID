import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import type { z } from "zod";
import type { remoteMachinePlatformReadinessSchema } from "@/schemas/machines";

export function RemoteMachinePlatformReadiness({
  platform,
  grouped = false,
}: {
  readonly grouped?: boolean;
  readonly platform?: z.infer<typeof remoteMachinePlatformReadinessSchema>;
}) {
  useUiTranslation();
  return (
    <div className={grouped ? "space-y-1" : "space-y-2 text-14 text-muted-foreground"} role="status">
      <p>{translateUi("Platform configuration:")}{" "}
        {platform?.configuration_ready === true
          ? translateUi("Ready for admission checks")
          : translateUi("Unavailable or not verified")}
        .
      </p>
      {platform?.blockers.length ? (
        <ul className="list-disc pl-5">
          {platform.blockers.map((blocker) => (
            <li key={blocker}>{blocker.replaceAll("_", " ")}</li>
          ))}
        </ul>
      ) : null}
      <p>{translateUi("Kubernetes native lifecycle observations confirm normal operations. Uncertain prior runtime state keeps recovery pending. This does not guarantee physical shutdown during a control-plane or node partition.")}</p>
    </div>
  );
}
