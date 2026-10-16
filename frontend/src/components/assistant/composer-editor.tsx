import {
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  type HTMLAttributes,
  type Ref,
} from "react";
import { activeMention } from "@/lib/assistant/nyxbot-mentions";
import { Editor } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import HardBreak from "@tiptap/extension-hard-break";
import { UndoRedo } from "@tiptap/extensions";
import { EditorState } from "@tiptap/pm/state";
import { stripAttachmentMention } from "@/lib/assistant/attachment-mentions";
import { Slice } from "@tiptap/pm/model";
import { closeHistory } from "@tiptap/pm/history";
import {
  composerMention,
  editorDocument,
  editorText,
  selectEditorOffsets,
  MAX_COMPOSER_LENGTH,
  type ComposerToken,
} from "./composer-editor-model";

export interface ComposerSelection {
  value: string;
  selectionStart: number;
  selectionEnd: number;
  mentionQueryAllowed?: boolean;
}
export interface ComposerEditorHandle extends ComposerSelection {
  readonly element: HTMLElement;
  readonly disabled: boolean;
  focus(): void;
  setSelectionRange(start: number, end: number): void;
}
export interface ComposerEditorProps extends Omit<
  HTMLAttributes<HTMLDivElement>,
  "onChange" | "onSelect" | "children"
> {
  ref?: Ref<ComposerEditorHandle>;
  value: string;
  disabled: boolean;
  placeholder: string;
  tokens: readonly ComposerToken[];
  expanded: boolean;
  onChange: (value: string) => void;
  onSelectionChange: (selection: ComposerSelection) => void;
}

function mentionQueryAllowed(editor: Editor): boolean {
  const before = editor.state.selection.$from.nodeBefore;
  return Boolean(
    before?.isText && activeMention(before.text!, before.text!.length),
  );
}

/** The editor owns selection/history; the composer keeps its existing plain-text transport and drafts. */
export function ComposerEditor({
  ref,
  value,
  tokens,
  disabled,
  placeholder,
  expanded,
  onChange,
  onSelectionChange,
  className,
  style,
  ...events
}: ComposerEditorProps) {
  const latest = useRef({
    value,
    tokens,
    disabled,
    onChange,
    onSelectionChange,
  });
  latest.current = { value, tokens, disabled, onChange, onSelectionChange };
  const container = useRef<HTMLDivElement>(null);
  const editorRef = useRef<Editor | null>(null);
  useLayoutEffect(() => {
    const editor = new Editor({
      element: container.current!,
      extensions: [
        Document,
        Paragraph.extend({ whitespace: "pre" }),
        Text,
        HardBreak,
        UndoRedo,
        composerMention(() => latest.current.tokens),
      ],
      content: editorDocument(latest.current.value, latest.current.tokens),
      editable: !latest.current.disabled,
      parseOptions: { preserveWhitespace: "full" },
      editorProps: {
        attributes: {
          "data-composer-textbox": "",
          role: "textbox",
          "aria-multiline": "true",
        },
        clipboardTextSerializer: (slice) =>
          slice.content.textBetween(0, slice.content.size, "\n", (node) =>
            node.type.name === "composerMention"
              ? `@${String(node.attrs.handle)}`
              : "\n",
          ),
        handlePaste: (view, event) => {
          if (event.defaultPrevented) return true;
          const text = event.clipboardData?.getData("text/plain");
          if (text === undefined) return false;
          const selectedLength =
            editorText(view.state.doc, view.state.selection.to).length -
            editorText(view.state.doc, view.state.selection.from).length;
          const room =
            MAX_COMPOSER_LENGTH -
            editorText(view.state.doc).length +
            selectedLength;
          const document = view.state.schema.nodeFromJSON(
            editorDocument(
              text.slice(0, Math.max(0, room)).replace(/\r\n?/g, "\n"),
              latest.current.tokens,
            ),
          );
          view.dispatch(
            view.state.tr.replaceSelection(
              new Slice(document.firstChild!.content, 0, 0),
            ),
          );
          return true;
        },
      },
      onUpdate: ({ editor }) =>
        latest.current.onChange(editorText(editor.state.doc)),
      onSelectionUpdate: ({ editor }) =>
        latest.current.onSelectionChange({
          value: editorText(editor.state.doc),
          mentionQueryAllowed: mentionQueryAllowed(editor),
          selectionStart: editorText(
            editor.state.doc,
            editor.state.selection.from,
          ).length,
          selectionEnd: editorText(editor.state.doc, editor.state.selection.to)
            .length,
        }),
    });
    editorRef.current = editor;
    return () => {
      editor.destroy();
      editorRef.current = null;
    };
  }, []);
  useImperativeHandle(
    ref,
    () => ({
      get element() {
        return editorRef.current?.view.dom ?? container.current!;
      },
      get value() {
        return editorRef.current ? editorText(editorRef.current.state.doc) : "";
      },
      get selectionStart() {
        const editor = editorRef.current;
        return editor
          ? editorText(editor.state.doc, editor.state.selection.from).length
          : 0;
      },
      get selectionEnd() {
        const editor = editorRef.current;
        return editor
          ? editorText(editor.state.doc, editor.state.selection.to).length
          : 0;
      },
      get mentionQueryAllowed() {
        return editorRef.current
          ? mentionQueryAllowed(editorRef.current)
          : false;
      },
      get disabled() {
        return !editorRef.current?.isEditable;
      },
      focus() {
        editorRef.current?.view.focus();
      },
      setSelectionRange(start, end) {
        const editor = editorRef.current;
        if (editor)
          editor.view.dispatch(
            editor.state.tr.setSelection(
              selectEditorOffsets(editor.state.doc, start, end),
            ),
          );
      },
    }),
    [],
  );
  const tokenKey = JSON.stringify(tokens);
  const previousTokenKey = useRef(tokenKey);
  const previousTokens = useRef(tokens);
  useLayoutEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    const changedTokens = previousTokenKey.current !== tokenKey;
    previousTokenKey.current = tokenKey;
    const removed = previousTokens.current.filter(
      (token) =>
        token.kind !== "agent" &&
        !latest.current.tokens.some((next) => next.id === token.id),
    );
    previousTokens.current = latest.current.tokens;
    if (value === editorText(editor.state.doc) && !changedTokens) return;
    const caret = editorText(
      editor.state.doc,
      editor.state.selection.from,
    ).length;
    const cleaned = removed.reduce(
      (text, token) => stripAttachmentMention(text, token.handle),
      value,
    );
    const doc = editor.schema.nodeFromJSON(
      editorDocument(cleaned, latest.current.tokens),
    );
    if (!value || removed.length) {
      // Sending and removing an upload must not leave a stale reference in undo history.
      editor.view.updateState(
        EditorState.create({
          doc,
          plugins: editor.state.plugins,
          selection: selectEditorOffsets(doc, Math.min(caret, cleaned.length)),
        }),
      );
      if (cleaned !== value) latest.current.onChange(cleaned);
      return;
    }
    const tr = closeHistory(editor.state.tr).replaceWith(
      0,
      editor.state.doc.content.size,
      doc.content,
    );
    tr.setSelection(selectEditorOffsets(tr.doc, Math.min(caret, value.length)));
    if (changedTokens) tr.setMeta("addToHistory", false);
    editor.view.dispatch(tr);
    editor.view.dispatch(closeHistory(editor.state.tr));
  }, [value, tokenKey]);
  useLayoutEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    if (editor.isEditable === disabled) editor.setEditable(!disabled, false);
    const root = editor.view.dom;
    const classes = `${className ?? ""} composer-rich-editor ProseMirror`;
    if (root.className !== classes) root.className = classes;
    root.style.whiteSpace = expanded ? "pre-wrap" : "pre";
    root.style.transitionDuration = style?.transitionDuration ?? "";
    root.dataset.placeholder = placeholder;
    root.setAttribute("aria-label", events["aria-label"] ?? placeholder);
    root.setAttribute("aria-disabled", String(disabled));
    root.setAttribute("role", events.role ?? "textbox");
    for (const name of [
      "aria-autocomplete",
      "aria-expanded",
      "aria-controls",
      "aria-activedescendant",
    ] as const) {
      const attr = events[name];
      if (attr === undefined) root.removeAttribute(name);
      else root.setAttribute(name, String(attr));
    }
    if (disabled && root === document.activeElement) root.blur();
  });
  return (
    <div
      ref={container}
      className="min-w-0"
      onKeyDownCapture={events.onKeyDown}
      onBlur={events.onBlur}
      onCompositionStart={events.onCompositionStart}
      onCompositionEnd={events.onCompositionEnd}
      onScrollCapture={events.onScroll}
    />
  );
}
