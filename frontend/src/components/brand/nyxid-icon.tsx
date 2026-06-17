export function NyxidIcon({
  className = "h-5 w-5",
}: {
  readonly className?: string;
}) {
  return (
    <img src="/nyxid-coloured-icon.svg" alt="NyxID" className={className} />
  );
}
