import {
  useImperativeHandle,
  useRef,
  type TextareaHTMLAttributes,
} from "react";
import type { ComposerEditorProps } from "./composer-editor";

/** Parent policy tests use a native input; browser tests exercise the real editor. */
export function ComposerEditor({
  ref,
  tokens: _tokens,
  expanded: _expanded,
  onChange,
  onSelectionChange,
  ...props
}: ComposerEditorProps) {
  void _tokens;
  void _expanded;
  const input = useRef<HTMLTextAreaElement>(null);
  useImperativeHandle(ref, () => ({
    get element() {
      return input.current!;
    },
    get value() {
      return input.current!.value;
    },
    get selectionStart() {
      return input.current!.selectionStart;
    },
    get selectionEnd() {
      return input.current!.selectionEnd;
    },
    get disabled() {
      return input.current!.disabled;
    },
    focus() {
      input.current!.focus();
    },
    insertDictation(text, selection) {
      const element = input.current!;
      if (element.value !== selection.value) return false;
      const next =
        element.value.slice(0, selection.selectionStart) +
        text +
        element.value.slice(selection.selectionEnd);
      onChange(next);
      return true;
    },
    setSelectionRange(start, end) {
      input.current!.setSelectionRange(start, end);
    },
  }));
  return (
    <textarea
      {...(props as TextareaHTMLAttributes<HTMLTextAreaElement>)}
      ref={input}
      onChange={(event) => {
        onChange(event.target.value);
        onSelectionChange(event.target);
      }}
      onSelect={(event) => onSelectionChange(event.currentTarget)}
    />
  );
}
