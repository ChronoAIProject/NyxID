import ts from "typescript";

/** Brands, algorithm names and protocol examples retain their spelling in every locale. */
export const NON_TRANSLATABLE_TOKENS = new Set([
  "NyxID", "NyxBot", "Telegram", "Claude Code", "Cursor", "Codex", "OpenClaw",
  "Google AI", "NyxID CLI", "YYYY-MM-DDTHH:mm:ssZ", "ABCD-EFGH", "SHA-256", "IP:",
]);

export type UntranslatedLiteral = {
  start: number;
  end: number;
  phrase: string;
  jsx: boolean;
};
/** Scan authored presentation sinks; interpolated data and protocol tokens are untouched. */
export function untranslatedLiterals(
  source: string,
  filename = "screen.tsx",
): UntranslatedLiteral[] {
  const file = ts.createSourceFile(
    filename,
    source,
    ts.ScriptTarget.Latest,
    true,
  );
  const result: UntranslatedLiteral[] = [];
  const visible = (text: string) =>
    /[A-Za-z]/.test(text) &&
    !NON_TRANSLATABLE_TOKENS.has(text.trim()) &&
    !/^(?:https?:\/\/|nyxid:\/\/|\/|\{id\}$|[\w.+-]+@[\w.-]+\.[a-z]+$)/i.test(
      text,
    );

  function literals(node: ts.Node, jsx = false) {
    if (ts.isJsxElement(node) || ts.isJsxSelfClosingElement(node)) {
      visit(node);
      return;
    }
    if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
      const name = node.expression.getText(file);
      if (
        /^(?:t|translate|translateApp)$|\.t$|(?:toLocale(?:Date|Time)?String|DateTimeFormat|RelativeTimeFormat|NumberFormat)$/.test(
          name,
        )
      )
        return;
      for (const arg of node.arguments ?? []) {
        // Closed protocol identifiers passed to a formatter are data, not copy.
        if (
          name !== "String" &&
          ((ts.isStringLiteral(arg) && /^[a-z][a-z0-9_-]*$/.test(arg.text)) ||
            (ts.isBinaryExpression(arg) &&
              arg.operatorToken.kind === ts.SyntaxKind.QuestionQuestionToken &&
              ts.isStringLiteral(arg.right) &&
              /^[a-z][a-z0-9_-]*$/.test(arg.right.text)))
        )
          continue;
        literals(arg);
      }
      return;
    }
    if (ts.isTemplateExpression(node)) {
      const phrase =
        node.head.text +
        node.templateSpans
          .map((span, index) => "{{expression" + index + "}}" + span.literal.text)
          .join("");
      if (visible(phrase.replace(/\{\{\w+\}\}/g, "")))
        result.push({
          start: node.getStart(file),
          end: node.end,
          phrase,
          jsx: false,
        });
      node.templateSpans.forEach((span) => literals(span.expression));
      return;
    }
    if (ts.isConditionalExpression(node)) {
      literals(node.whenTrue);
      literals(node.whenFalse);
      return;
    }
    if (ts.isStringLiteralLike(node) || ts.isJsxText(node)) {
      const text = ts.isJsxText(node)
        ? node.text.replace(/\s+/g, " ").trim()
        : node.text;
      if (visible(text))
        result.push({
          start: node.getStart(file),
          end: node.end,
          phrase: text,
          jsx: jsx || ts.isJsxText(node),
        });
      return;
    }
    // Only literal copy in an expression: property names, types and enum comparisons are not copy.
    if (
      ts.isPropertyAccessExpression(node) ||
      ts.isElementAccessExpression(node) ||
      ts.isTypeNode(node)
    )
      return;
    if (
      ts.isBinaryExpression(node) &&
      [
        ts.SyntaxKind.EqualsEqualsEqualsToken,
        ts.SyntaxKind.ExclamationEqualsEqualsToken,
      ].includes(node.operatorToken.kind)
    )
      return;
    ts.forEachChild(node, (child) => literals(child));
  }
  function visit(node: ts.Node) {
    if (ts.isJsxAttribute(node) && node.name.getText(file) === "message" &&
      (ts.isJsxSelfClosingElement(node.parent.parent) || ts.isJsxOpeningElement(node.parent.parent)) &&
      node.parent.parent.tagName.getText(file) === "Translation") return;
    if (
      ts.isJsxAttribute(node) &&
      [
        "accessibilityLabel",
        "accessibilityHint",
        "placeholder",
        "label",
        "title",
        "subtitle",
        "description",
        "message",
        "footer",
        "body",
        "badge",
        "caption",
        "detail",
        "emptyMessage",
        "emptyText",
        "searchPlaceholder",
        "confirmLabel",
        "cancelLabel",
        "closeLabel",
        "createLabel",
        "refreshLabel",
        "helperText",
        "errorMessage",
        "fallbackLabel",
        "primaryLabel",
        "secondaryLabel",
        "buttonLabel",
      ].includes(node.name.getText(file)) &&
      node.initializer
    ) {
      literals(node.initializer, ts.isStringLiteral(node.initializer));
      return;
    }
    if (
      ts.isJsxElement(node) &&
      /^(?:Text|Label|DynamicTypeText|Animated\.Text)$/.test(
        node.openingElement.tagName.getText(file),
      )
    ) {
      for (const child of node.children) {
        if (ts.isJsxExpression(child) && child.expression)
          literals(child.expression);
        else if (ts.isJsxText(child)) literals(child, true);
        else visit(child);
      }
      visit(node.openingElement);
      return;
    }
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(file) === "Alert.alert"
    ) {
      node.arguments.slice(0, 2).forEach((arg) => literals(arg));
      const buttons = node.arguments[2];
      if (buttons) {
        const button = (item: ts.Node) => {
          if (
            ts.isPropertyAssignment(item) &&
            item.name.getText(file) === "text"
          )
            literals(item.initializer);
          else ts.forEachChild(item, button);
        };
        button(buttons);
      }
      return;
    }
    ts.forEachChild(node, visit);
  }
  visit(file);
  return result.filter(
    (item, i, all) => all.findIndex((x) => x.start === item.start) === i,
  );
}
