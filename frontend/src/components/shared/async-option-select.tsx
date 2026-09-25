import {
  useEffect,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import { Anchor } from "@radix-ui/react-popover";
import { ChevronDown, ChevronRight, X } from "lucide-react";
import { useOptions } from "@/hooks/use-options";
import { useAuthStore } from "@/stores/auth-store";
import { Popover, PopoverContent } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  type OptionItem,
  type OptionsContext,
  type OptionSet,
} from "@/types/options";

type Props = {
  readonly optionSet: OptionSet;
  readonly context: OptionsContext;
  readonly value: readonly string[];
  readonly onChange: (values: string[]) => void;
  readonly label: string;
  readonly disabled?: boolean;
  readonly allowCustom?: boolean;
  readonly delimiter?: string;
} & Pick<
  ComponentProps<"input">,
  "id" | "onBlur" | "aria-describedby" | "aria-invalid" | "ref"
>;

type Choice = {
  kind: "branch" | "leaf";
  value: string;
  label?: string;
  disabled?: boolean;
};
const choiceKey = (choice: Choice) => `${choice.kind}:${choice.value}`;

// Every branch is a prefix of an actual returned value. Segments are never
// combined across values, and only complete values become leaf suggestions.
function suggestions(
  items: OptionItem[],
  draft: string,
  delimiter?: string,
): Choice[] {
  const choices = new Map<string, Choice>();
  const split = delimiter ? draft.lastIndexOf(delimiter) : -1;
  const prefix = split < 0 ? "" : draft.slice(0, split + delimiter!.length);
  const fragment = draft.slice(prefix.length).toLowerCase();
  for (const item of items) {
    const fullMatch =
      item.value.toLowerCase().includes(draft.toLowerCase()) ||
      item.label.toLowerCase().includes(draft.toLowerCase());
    let choice: Choice | undefined;
    if (
      delimiter &&
      item.value.toLowerCase().startsWith(prefix.toLowerCase())
    ) {
      const rest = item.value.slice(prefix.length);
      const next = rest.indexOf(delimiter);
      if (next >= 0 && rest.slice(0, next).toLowerCase().startsWith(fragment)) {
        choice = {
          kind: "branch",
          value: item.value.slice(0, prefix.length + next + delimiter.length),
        };
      } else if (fullMatch) {
        choice = {
          kind: "leaf",
          value: item.value,
          label: item.label,
          disabled: item.disabled,
        };
      }
    } else if (!prefix && fullMatch) {
      choice = {
        kind: "leaf",
        value: item.value,
        label: item.label,
        disabled: item.disabled,
      };
    }
    if (choice) choices.set(choiceKey(choice), choice);
  }
  return [...choices.values()];
}

export function AsyncOptionSelect(props: Props) {
  const identity = useAuthStore((state) => state.user?.id);
  const [session, setSession] = useState({
    identity,
    optionSet: props.optionSet,
    context: props.context,
    generation: 0,
  });
  const oldScope =
    session.context.kind === "service-scope" ? session.context : undefined;
  const newScope =
    props.context.kind === "service-scope" ? props.context : undefined;
  const sameResource =
    session.optionSet === props.optionSet &&
    session.context.kind === props.context.kind &&
    oldScope?.principal_type === newScope?.principal_type &&
    oldScope?.service_account_id === newScope?.service_account_id;
  const sameOwner = oldScope?.owner_id === newScope?.owner_id;
  let generation = session.generation;
  if (session.identity !== identity || !sameResource || !sameOwner) {
    // Initial identity/owner loading enables suggestions without interrupting
    // an editable draft. Actual account or owner switches still reset the UI.
    const loadingIdentity =
      session.identity === undefined && identity !== undefined;
    const loadingPersonalOwner =
      newScope !== undefined &&
      !oldScope?.owner_id &&
      newScope.owner_id === identity &&
      (loadingIdentity || session.identity === identity);
    const initialLoad =
      sameResource && ((sameOwner && loadingIdentity) || loadingPersonalOwner);
    if (!initialLoad) generation += 1;
    setSession({
      identity,
      optionSet: props.optionSet,
      context: props.context,
      generation,
    });
  }
  return <OptionSelection key={generation} {...props} />;
}

type Draft = {
  base: string[];
  text: string;
  original?: string;
  branch: boolean;
  expected: string[];
};

function replaceSelection(
  base: readonly string[],
  original: string | undefined,
  additions: string[],
) {
  return [
    ...new Set(
      original === undefined
        ? [...base, ...additions]
        : base.flatMap((value) => (value === original ? additions : [value])),
    ),
  ];
}

function OptionSelection({
  optionSet,
  context,
  value,
  onChange,
  label,
  disabled,
  allowCustom = false,
  delimiter,
  ref: forwardedRef,
  ...inputProps
}: Props) {
  const generatedId = useId();
  const id = inputProps.id ?? generatedId;
  const listId = `${id}-options`;
  const input = useRef<HTMLInputElement>(null);
  const field = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const focusAfter = useRef<{ value?: string; select?: boolean } | null>(null);
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<Draft | null>(null);
  // Controlled updates from the parent acknowledge a draft. An external reset
  // takes precedence; do not retain another form's pending edit.
  const editing =
    pending && JSON.stringify(value) === JSON.stringify(pending.expected)
      ? pending
      : null;
  if (pending && !editing) setPending(null);
  const selected = [...new Set(editing?.base ?? value)];
  const draft = editing?.text ?? "";
  const original = editing?.original;
  const [active, setActive] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  useImperativeHandle(forwardedRef, () => input.current!);
  useLayoutEffect(() => {
    const target = focusAfter.current;
    if (!target) return;
    focusAfter.current = null;
    if (target.value !== undefined) {
      const button = [
        ...(field.current?.querySelectorAll<HTMLButtonElement>(
          "[data-scope-edit]",
        ) ?? []),
      ].find((entry) => entry.dataset.scopeEdit === target.value);
      button?.focus();
    } else {
      input.current?.focus();
      if (target.select) input.current?.select();
    }
  });
  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(draft.slice(0, 200)), 250);
    return () => window.clearTimeout(timer);
  }, [draft]);
  const query = useOptions(
    optionSet,
    context,
    context.kind === "service-history" ? "" : search,
  );
  const { hasNextPage, isFetching, isError, fetchNextPage } = query;
  useEffect(() => {
    if (open && hasNextPage && !isFetching && !isError) void fetchNextPage();
  }, [open, hasNextPage, isFetching, isError, fetchNextPage]);
  const pages =
    query.isError && !query.retainPartialData ? [] : (query.data?.pages ?? []);
  const items = [
    ...new Map(
      pages.flatMap((page) => page.items).map((item) => [item.value, item]),
    ).values(),
  ];
  // Filter complete values before building prefixes, so a branch remains only
  // while at least one actual, unselected descendant is available.
  const choices = suggestions(
    items.filter((item) => !selected.includes(item.value)),
    draft,
    delimiter,
  );
  const activeIndex = choices.findIndex(
    (choice) => choiceKey(choice) === active,
  );
  useEffect(() => {
    if (activeIndex >= 0)
      menu.current
        ?.querySelector(`[data-option-index="${activeIndex}"]`)
        ?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex]);

  function updateDraft(text: string, branch = false) {
    if (disabled) return;
    const base = editing?.base ?? [...value];
    const expected =
      !branch && allowCustom
        ? replaceSelection(base, original, text.split(/\s+/).filter(Boolean))
        : base;
    setPending({ base, original, text, branch, expected });
    // Like a normal text input, the form receives the latest typed value now.
    // Blur only closes the popup; it never moves a Save button by adding rows.
    if (JSON.stringify(expected) !== JSON.stringify(value)) onChange(expected);
    setActive(null);
    setOpen(true);
  }
  function finish(additions: string[]) {
    if (disabled) return;
    onChange(replaceSelection(selected, original, additions));
    focusAfter.current = {};
    setPending(null);
    setActive(null);
  }
  function commitDraft() {
    if (!disabled && allowCustom && editing && !editing.branch)
      finish(draft.split(/\s+/).filter(Boolean));
  }
  function cancelEdit() {
    if (disabled || !editing) return;
    onChange(editing.base);
    focusAfter.current = { value: original };
    setPending(null);
    setActive(null);
    setOpen(false);
  }
  function edit(valueToEdit: string) {
    if (disabled) return;
    // Finish an existing typed draft before beginning a different pill edit.
    const base = [...value];
    focusAfter.current = { select: true };
    setPending({
      base,
      original: valueToEdit,
      text: valueToEdit,
      branch: false,
      expected: [...value],
    });
    setActive(null);
    setOpen(true);
  }
  function remove(valueToRemove: string) {
    if (disabled) return;
    const next = valueToRemove === original ? selected : [...value];
    onChange(next.filter((entry) => entry !== valueToRemove));
    setPending(null);
    setActive(null);
  }
  function choose(choice: Choice) {
    if (disabled || choice.disabled) return;
    if (choice.kind === "branch") updateDraft(choice.value, true);
    else finish([choice.value]);
    input.current?.focus();
  }
  function navigate(direction: number) {
    if (disabled) return;
    setOpen(true);
    if (!choices.length) return;
    let next = activeIndex < 0 && direction < 0 ? 0 : activeIndex;
    for (let count = 0; count < choices.length; count++) {
      next = (next + direction + choices.length) % choices.length;
      if (!choices[next]!.disabled) {
        setActive(choiceKey(choices[next]!));
        break;
      }
    }
  }
  function leaveWidget(target: EventTarget | null) {
    if (
      !(target instanceof Node) ||
      (!field.current?.contains(target) && !menu.current?.contains(target))
    )
      setOpen(false);
  }

  const editor = (
    <input
      {...inputProps}
      ref={input}
      id={id}
      role="combobox"
      aria-label={label}
      aria-expanded={open}
      aria-controls={listId}
      aria-autocomplete="list"
      aria-haspopup="listbox"
      aria-activedescendant={
        open && activeIndex >= 0 ? `${id}-choice-${activeIndex}` : undefined
      }
      autoComplete="off"
      className="h-6 min-w-20 flex-1 bg-transparent p-0 text-[12px] text-foreground outline-none placeholder:text-text-tertiary disabled:cursor-not-allowed"
      size={original === undefined ? undefined : Math.max(10, draft.length + 1)}
      placeholder={
        allowCustom
          ? selected.length
            ? "Add value…"
            : "Select or type…"
          : "Search choices…"
      }
      disabled={disabled}
      value={draft}
      onFocus={() => setOpen(true)}
      onClick={() => setOpen(true)}
      onBlur={inputProps.onBlur}
      onChange={(event) => updateDraft(event.target.value)}
      onPaste={(event) => {
        if (!allowCustom || disabled) return;
        const pasted = event.clipboardData.getData("text");
        if (/\s/.test(pasted)) {
          event.preventDefault();
          const start = event.currentTarget.selectionStart ?? draft.length;
          const end = event.currentTarget.selectionEnd ?? start;
          finish(
            (draft.slice(0, start) + pasted + draft.slice(end))
              .split(/\s+/)
              .filter(Boolean),
          );
        }
      }}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault();
          navigate(event.key === "ArrowDown" ? 1 : -1);
        } else if (event.key === "Enter") {
          event.preventDefault();
          if (open && activeIndex >= 0) choose(choices[activeIndex]!);
          else commitDraft();
        } else if (event.key === "Escape" && (open || original !== undefined)) {
          event.preventDefault();
          event.stopPropagation();
          if (original !== undefined) cancelEdit();
          else {
            setOpen(false);
            setActive(null);
          }
        }
      }}
    />
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Anchor asChild>
        <div
          ref={field}
          onBlur={(event) => leaveWidget(event.relatedTarget)}
          className={cn(
            "flex min-h-8 w-full min-w-0 flex-wrap items-center gap-1 rounded-lg border border-input bg-transparent px-3 py-0.5 text-foreground transition-colors duration-200 focus-within:border-white/[0.15] has-[[aria-invalid=true]]:border-destructive",
            disabled && "opacity-50",
          )}
          onClick={(event) => {
            if (!disabled && event.target === event.currentTarget) {
              input.current?.focus();
              setOpen(true);
            }
          }}
        >
          {selected.map((v) => (
            <div
              key={v}
              role="group"
              aria-label={`Selected ${v}`}
              className={cn(
                "flex min-h-6 max-w-full items-stretch overflow-hidden rounded-md border border-input bg-muted/25 text-[12px]",
                original === v && "border-ring ring-1 ring-ring",
              )}
            >
              {optionSet !== "service-scope" ? (
                <span className="min-w-0 break-words px-2 py-0.5">
                  {pages
                    .flatMap((page) => [...page.items, ...page.selected_items])
                    .find((item) => item.value === v)?.label ?? v}
                </span>
              ) : original === v ? (
                editor
              ) : (
                <button
                  type="button"
                  data-scope-edit={v}
                  disabled={disabled}
                  aria-label={`Edit ${v}`}
                  className="min-w-0 break-all px-2 py-0.5 text-left font-mono outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                  onClick={() => edit(v)}
                >
                  {v}
                </button>
              )}
              <button
                type="button"
                disabled={disabled}
                aria-label={`Remove ${v}`}
                className="flex w-6 shrink-0 items-center justify-center border-l border-border/70 text-text-tertiary outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                onPointerDown={(event) => event.preventDefault()}
                onClick={() => remove(v)}
              >
                <X className="size-3.5" aria-hidden="true" />
              </button>
            </div>
          ))}
          <div
            className={cn(
              "flex items-center gap-2",
              original === undefined ? "min-w-36 flex-1" : "ml-auto",
            )}
          >
            {original === undefined && editor}
            <button
              type="button"
              tabIndex={-1}
              disabled={disabled}
              className="flex h-6 shrink-0 items-center rounded text-text-tertiary"
              aria-label={`Toggle ${label.toLowerCase()} suggestions`}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => {
                setOpen(!open);
                if (!open) input.current?.focus();
              }}
            >
              <ChevronDown className="size-3.5" aria-hidden="true" />
            </button>
          </div>
        </div>
      </Anchor>
      <PopoverContent
        role="presentation"
        ref={menu}
        onBlur={(event) => leaveWidget(event.relatedTarget)}
        align="start"
        className="w-[var(--radix-popover-trigger-width)] min-w-56 max-w-[calc(100vw-2rem)] p-1.5"
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onInteractOutside={(event) => {
          if (field.current?.contains(event.target as Node))
            event.preventDefault();
        }}
        onEscapeKeyDown={(event) => {
          event.preventDefault();
          event.stopPropagation();
          if (original !== undefined) cancelEdit();
          else {
            setOpen(false);
            input.current?.focus();
          }
        }}
      >
        {original !== undefined && (
          <p className="border-b px-2 py-2 text-xs text-muted-foreground">
            Editing value. Enter saves; Escape cancels.
          </p>
        )}
        {query.isFetching && (
          <p role="status" className="px-3 py-2 text-xs text-muted-foreground">
            {pages.length
              ? "Loading remaining suggestions…"
              : "Loading suggestions…"}
          </p>
        )}
        {query.isError && !query.versionChanged && (
          <div role="alert" className="px-3 py-2 text-xs">
            <p className="text-destructive">
              {pages.length
                ? "Some suggestions could not load."
                : "Suggestions unavailable."}
              {allowCustom ? " Custom values still work." : ""}
            </p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onPointerDown={(event) => event.preventDefault()}
              onClick={() =>
                void (query.isFetchNextPageError
                  ? query.fetchNextPage()
                  : query.refetch())
              }
            >
              Retry suggestions
            </Button>
          </div>
        )}
        <div
          id={listId}
          role="listbox"
          aria-label={`${label} suggestions`}
          aria-multiselectable="true"
          className="max-h-64 overflow-y-auto overscroll-contain"
        >
          {choices.map((choice, index) => (
            <div
              key={choiceKey(choice)}
              id={`${id}-choice-${index}`}
              role="option"
              aria-label={
                choice.kind === "branch"
                  ? `Explore ${choice.value}`
                  : optionSet === "service-scope"
                    ? choice.value
                    : (choice.label ?? choice.value)
              }
              aria-selected="false"
              aria-disabled={disabled || choice.disabled}
              data-option-index={index}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-md px-3 py-1.5 text-[12px] transition-colors duration-200 hover:bg-white/[0.06]",
                activeIndex === index && "bg-white/[0.06]",
                (disabled || choice.disabled) &&
                  "cursor-not-allowed opacity-50",
              )}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => choose(choice)}
            >
              <span className="min-w-0 flex-1">
                <span
                  className={cn(
                    "block break-words text-xs",
                    optionSet === "service-scope" && "font-mono",
                  )}
                >
                  {optionSet === "service-scope"
                    ? choice.value
                    : (choice.label ?? choice.value)}
                </span>
                {optionSet === "service-scope" &&
                  choice.label &&
                  choice.label !== choice.value && (
                    <span className="block break-words text-xs text-muted-foreground">
                      {choice.label}
                    </span>
                  )}
              </span>
              {choice.kind === "branch" && (
                <ChevronRight className="size-4 shrink-0" aria-hidden="true" />
              )}
            </div>
          ))}
        </div>
        {!choices.length && !query.isFetching && query.isSuccess && (
          <p role="status" className="px-3 py-2 text-xs text-muted-foreground">
            No matching suggestions.
          </p>
        )}
        {allowCustom && draft.trim() && !editing?.branch && (
          <p className="border-t px-2 py-2 text-xs text-muted-foreground">
            Press Enter to finish this scope.
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
