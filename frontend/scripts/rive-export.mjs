/** Validate the documented publish result without selecting or changing a workspace. */
export function validateRiveExport(report, expectedWorkspace) {
  if (
    report.success !== true ||
    typeof report.data?.riv !== "string" ||
    !report.data.riv
  )
    throw Error("Rive runtime export failed or returned no runtime file.");
  if (
    report.warnings != null &&
    (!Array.isArray(report.warnings) || report.warnings.length)
  )
    throw Error(
      "Rive export returned warnings; review them before accepting the asset.",
    );
  const workspace = report.workspace;
  if (
    typeof workspace?.id !== "string" ||
    !workspace.id ||
    typeof workspace.name !== "string" ||
    !workspace.name ||
    typeof workspace.personal !== "boolean"
  )
    throw Error("Rive export did not identify its workspace.");
  if (expectedWorkspace && workspace.id !== expectedWorkspace)
    throw Error(
      "Rive export used a different workspace than --workspace expected.",
    );
  return {
    id: workspace.id,
    name: workspace.name,
    personal: workspace.personal,
  };
}
