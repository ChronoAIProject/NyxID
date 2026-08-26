export function NyxidIcon({
  className = "h-5 w-5",
  alt = "NyxID",
}: {
  readonly className?: string;
  readonly alt?: string;
}) {
  return (
    <img src="/nyxid-coloured-icon.svg" alt={alt} className={className} />
  );
}
