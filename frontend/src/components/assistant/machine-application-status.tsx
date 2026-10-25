import type { MachineContextReview } from "@/schemas/machine-access";
import type { MachineConfiguration } from "@/schemas/machines";

/** Both booleans are server observations; neither is inferred from a mutation. */
export function MachineApplicationStatus({
  authorizationApplied,
  environmentApplied,
  configuration,
  target,
}: {
  readonly authorizationApplied?: boolean;
  readonly environmentApplied?: boolean;
  readonly configuration: MachineConfiguration | null;
  readonly target?: MachineContextReview["machine_configuration_target"];
}) {
  const configApplied =
    configuration?.state === "applied" && configuration.reason === null;
  const targetMatches =
    !target ||
    Boolean(
      configuration &&
      target.revision === configuration.revision &&
      target.digest === configuration.digest &&
      target.runtime_generation === configuration.runtime_generation,
    );
  return (
    <section
      aria-label="Environment application status"
      className="space-y-2 text-12 text-muted-foreground"
    >
      {authorizationApplied !== undefined ? (
        <p role="status">
          {authorizationApplied
            ? "Agent authorization applied"
            : "Agent authorization application is not confirmed"}
        </p>
      ) : null}
      {configuration ? (
        <>
          <p
            role="status"
            className={configApplied ? "text-success" : "text-warning"}
          >
            {configApplied
              ? "Hosted configuration applied"
              : configuration.state === "pending"
                ? "Hosted configuration pending"
                : "Hosted configuration not confirmed"}
          </p>
          <p>
            Current hosted configuration revision {configuration.revision} ·
            config version {configuration.version} · runtime generation{" "}
            {configuration.runtime_generation}
          </p>
          <details>
            <summary className="cursor-pointer">
              Configuration evidence details
            </summary>
            <p className="mt-2 break-all font-mono text-11">
              Digest: {configuration.digest}
            </p>
            {configuration.reason ? (
              <p className="mt-1 break-words">
                {configuration.reason === "bootstrap_pending"
                  ? "Waiting for bootstrap configuration delivery."
                  : configuration.reason ===
                      "current_runtime_configuration_evidence_required"
                    ? "Waiting for authenticated configuration evidence from the current runtime generation."
                    : configuration.reason.replaceAll("_", " ")}
              </p>
            ) : null}
            <p className="mt-1">
              This is the server’s current configuration observation. It does
              not establish Agent separation support, physical stopping or
              accounting finality.
            </p>
          </details>
        </>
      ) : (
        <p>Hosted configuration evidence is not reported for this machine.</p>
      )}
      {!targetMatches ? (
        <p role="status" className="text-warning">
          The current hosted configuration differs from the reviewed target.
          Review current machine status before another mode change.
        </p>
      ) : null}
      {environmentApplied &&
      authorizationApplied &&
      configApplied &&
      targetMatches ? (
        <p role="status" className="text-success">
          Environment mode applied
        </p>
      ) : configuration && environmentApplied !== undefined ? (
        <p role="status">
          Environment mode application is not confirmed. Approval, saved
          permissions, authority expiry and a Node ACK alone do not confirm both
          application chains.
        </p>
      ) : null}
    </section>
  );
}
