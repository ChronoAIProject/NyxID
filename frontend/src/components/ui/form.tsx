import * as React from "react";
import type * as LabelPrimitive from "@radix-ui/react-label";
import { Slot } from "@radix-ui/react-slot";
import {
  Controller,
  type ControllerProps,
  type FieldPath,
  type FieldValues,
  FormProvider,
  useForm,
  type UseFormProps,
  type UseFormReturn,
  useFormContext,
  useFormState,
} from "react-hook-form";
import { cn } from "@/lib/utils";
import { firstNestedErrorMessage } from "@/lib/form-errors";
import { Label } from "@/components/ui/label";

const Form = FormProvider;

/**
 * Drop-in replacement for `useForm`. RHF's `setValue` leaves
 * `dirtyFields`/`touchedFields` untouched and skips validation unless every
 * call site remembers `{ shouldDirty, shouldTouch, shouldValidate }` — so
 * non-text controls wired via `form.watch()` + `form.setValue()` (Radix
 * Switch/Select/Checkbox, custom editors) never enable dirty-gated submit
 * buttons. This hook flips those defaults to `true`, matching what a
 * `Controller`-driven input reports. Programmatic writes (prefill,
 * normalization in effects) opt out per call with
 * `{ shouldDirty: false, shouldTouch: false }`.
 *
 * Caveats: each defaulted setValue runs one full schema parse (fine for
 * discrete controls; avoid for per-keystroke writes), and RHF's
 * useFieldArray branch of setValue ignores shouldTouch/shouldValidate —
 * no useFieldArray exists in this app today.
 */
function useAppForm<
  TFieldValues extends FieldValues = FieldValues,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- mirrors useForm's own TContext default
  TContext = any,
  TTransformedValues = TFieldValues,
>(
  props?: UseFormProps<TFieldValues, TContext, TTransformedValues>,
): UseFormReturn<TFieldValues, TContext, TTransformedValues> {
  const form = useForm<TFieldValues, TContext, TTransformedValues>(props);
  // useForm returns a stable object (only its formState property is swapped
  // each render), so this memo runs once. The Proxy shadows setValue while
  // every other read — including formState — stays live on the underlying
  // form; spreading (e.g. <Form {...form}>) also goes through the get trap.
  return React.useMemo(() => {
    const setValue: typeof form.setValue = (name, value, options) =>
      form.setValue(name, value, {
        shouldDirty: true,
        shouldTouch: true,
        shouldValidate: true,
        ...options,
      });
    return new Proxy(form, {
      get: (target, prop, receiver) =>
        prop === "setValue" ? setValue : Reflect.get(target, prop, receiver),
    });
  }, [form]);
}

/**
 * Renders next to a submit button after a blocked submit attempt: says how
 * many fields failed validation and quotes the first message. Field-level
 * highlights show WHERE the problem is; this shows WHY the click did
 * nothing — without it, errors inside collapsed sections or off-screen
 * rows make Save look broken. Clears automatically as fields revalidate.
 * Must be rendered inside `<Form {...form}>`.
 */
function FormSubmitErrors({ className }: { readonly className?: string }) {
  // useFormState subscribes independently of the parent page's formState
  // reads, so this re-renders even if the host never touches errors.
  const { submitCount, errors } = useFormState();
  const fieldErrors = Object.entries(errors).filter(([key]) => key !== "root");
  if (submitCount === 0 || fieldErrors.length === 0) return null;
  const first = firstNestedErrorMessage(Object.fromEntries(fieldErrors));
  const count = fieldErrors.length;
  return (
    <p role="alert" className={cn("text-xs text-destructive", className)}>
      Cannot save — {count} {count === 1 ? "field needs" : "fields need"}{" "}
      attention{first ? `: ${first}` : "."}
    </p>
  );
}

interface FormFieldContextValue<
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
> {
  readonly name: TName;
}

const FormFieldContext = React.createContext<FormFieldContextValue>(
  {} as FormFieldContextValue,
);

const FormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: ControllerProps<TFieldValues, TName>) => {
  const contextValue = React.useMemo(
    () => ({ name: props.name }),
    [props.name],
  );

  return (
    <FormFieldContext.Provider value={contextValue}>
      <Controller {...props} />
    </FormFieldContext.Provider>
  );
};

interface FormItemContextValue {
  readonly id: string;
}

const FormItemContext = React.createContext<FormItemContextValue>(
  {} as FormItemContextValue,
);

function useFormField() {
  const fieldContext = React.useContext(FormFieldContext);
  const itemContext = React.useContext(FormItemContext);
  const { getFieldState, formState } = useFormContext();

  const fieldState = getFieldState(fieldContext.name, formState);

  const { id } = itemContext;

  return {
    id,
    name: fieldContext.name,
    formItemId: `${id}-form-item`,
    formDescriptionId: `${id}-form-item-description`,
    formMessageId: `${id}-form-item-message`,
    ...fieldState,
  };
}

const FormItem = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => {
  const id = React.useId();
  const contextValue = React.useMemo(() => ({ id }), [id]);

  return (
    <FormItemContext.Provider value={contextValue}>
      <div ref={ref} className={cn("space-y-3", className)} {...props} />
    </FormItemContext.Provider>
  );
});
FormItem.displayName = "FormItem";

const FormLabel = React.forwardRef<
  React.ComponentRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => {
  const { error, formItemId } = useFormField();

  return (
    <Label
      ref={ref}
      className={cn(error && "text-destructive", className)}
      htmlFor={formItemId}
      {...props}
    />
  );
});
FormLabel.displayName = "FormLabel";

const FormControl = React.forwardRef<
  React.ComponentRef<typeof Slot>,
  React.ComponentPropsWithoutRef<typeof Slot>
>(({ ...props }, ref) => {
  const { error, formItemId, formDescriptionId, formMessageId } =
    useFormField();

  return (
    <Slot
      ref={ref}
      id={formItemId}
      aria-describedby={
        error ? `${formDescriptionId} ${formMessageId}` : formDescriptionId
      }
      aria-invalid={!!error}
      {...props}
    />
  );
});
FormControl.displayName = "FormControl";

const FormDescription = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => {
  const { formDescriptionId } = useFormField();

  return (
    <p
      ref={ref}
      id={formDescriptionId}
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  );
});
FormDescription.displayName = "FormDescription";

const FormMessage = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, children, ...props }, ref) => {
  const { error, formMessageId } = useFormField();
  // Array-container errors carry only nested per-index errors and no
  // `message`; String(undefined) would render a literal "undefined".
  const body = error?.message ? String(error.message) : children;

  if (!body) {
    return null;
  }

  return (
    <p
      ref={ref}
      id={formMessageId}
      className={cn("text-sm font-medium text-destructive", className)}
      {...props}
    >
      {body}
    </p>
  );
});
FormMessage.displayName = "FormMessage";

export {
  useAppForm,
  useFormField,
  Form,
  FormItem,
  FormLabel,
  FormControl,
  FormDescription,
  FormMessage,
  FormField,
  FormSubmitErrors,
};
