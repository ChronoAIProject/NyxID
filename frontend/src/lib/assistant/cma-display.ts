export function cmaReadableLabel(value: string): string {
  const label = value.replaceAll("_", " ").replaceAll(".", " · ");
  return label.charAt(0).toUpperCase() + label.slice(1);
}
