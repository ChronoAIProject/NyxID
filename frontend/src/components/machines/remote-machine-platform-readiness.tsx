import { useUiTranslation } from "@/i18n/use-ui-translation";
import type { z } from "zod";
import type { remoteMachinePlatformReadinessSchema } from "@/schemas/machines";

export function RemoteMachinePlatformReadiness({
  platform,
}: {
  readonly platform?: z.infer<typeof remoteMachinePlatformReadinessSchema>;
}) {
  const { t } = useUiTranslation();
  return (
    <div className="space-y-2 text-12 text-muted-foreground" role="status">
      <p>{t("Platform configuration:")}{" "}
        {platform?.configuration_ready === true
          ? t("Ready for admission checks")
          : t("Unavailable or not verified")}
        .
      </p>
      {platform?.blockers.length ? (
        <ul className="list-disc pl-5">
          {platform.blockers.map((blocker) => (
            <li key={blocker}>{blocker.replaceAll("_", " ")}</li>
          ))}
        </ul>
      ) : null}
      <p>{t("Kubernetes native lifecycle observations confirm normal operations. Uncertain prior runtime state keeps recovery pending. This does not guarantee physical shutdown during a control-plane or node partition.")}</p>
    </div>
  );
}
