import { TEXT_STYLES } from "@/lib/typography";
import * as React from "react";
import * as LabelPrimitive from "@radix-ui/react-label";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const labelVariants = cva(
  `${TEXT_STYLES.fieldLabel} unthemed:leading-none text-muted-foreground peer-disabled:cursor-not-allowed peer-disabled:opacity-70 unthemed:text-xs`,
);

const Label = React.forwardRef<
  React.ComponentRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root> &
    VariantProps<typeof labelVariants>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    data-field-label=""
    className={cn(labelVariants(), className)}
    {...props}
  />
));
Label.displayName = LabelPrimitive.Root.displayName;

/** Explanatory text is separate from the field caption, even in a wrapping label. */
function FieldHelp({ as: Component = "span", variant = "secondary", className, ...props }: React.ComponentPropsWithoutRef<"span"> & { as?: "span" | "p"; variant?: "secondary" | "body" | "emphasis" }) {
  const role = { secondary: TEXT_STYLES.secondary, body: TEXT_STYLES.body, emphasis: TEXT_STYLES.secondaryEmphasis }[variant];
  return <Component data-field-help="" className={cn(role, "text-muted-foreground", className)} {...props} />;
}

export { Label, FieldHelp };
