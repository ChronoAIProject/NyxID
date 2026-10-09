import type { ButtonHTMLAttributes, ReactNode } from "react";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly label: string;
  readonly children: ReactNode;
  readonly tone?: "default" | "active";
}

export function IconButton({
  label,
  children,
  className = "",
  tone = "default",
  ...props
}: IconButtonProps) {
  return (
    <button
      type="button"
      className={`icon-button icon-button--${tone} ${className}`.trim()}
      aria-label={label}
      title={label}
      {...props}
    >
      {children}
    </button>
  );
}
