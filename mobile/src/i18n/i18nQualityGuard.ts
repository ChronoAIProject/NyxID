import ts from "typescript";

const translationCall = (node: ts.Node, file: ts.SourceFile) =>
  ts.isCallExpression(node) && /^(?:t|translateApp|translate)$|\.t$/.test(node.expression.getText(file));

/** Import-time copy is frozen before LanguageProvider restores the app language. */
export function moduleScopeTranslations(source: string, filename = "screen.tsx") {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const names = new Set(["translateApp", "translate"]);
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) {
      for (const binding of bindings.elements) {
        if (names.has((binding.propertyName ?? binding.name).text))
          names.add(binding.name.text);
      }
    }
  }
  const failures: number[] = [];
  const visit = (node: ts.Node) => {
    // Getters and callbacks translate when invoked, rather than at import time.
    if (ts.isFunctionLike(node)) return;
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) &&
        names.has(node.expression.text))
      failures.push(file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1);
    ts.forEachChild(node, visit);
  };
  visit(file);
  return failures;
}

export function translationConcatenations(source: string, filename = "screen.tsx") {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const failures: number[] = [];
  const containsTranslation = (node: ts.Node): boolean => {
    if (translationCall(node, file)) return true;
    return node.getChildren(file).some(containsTranslation);
  };
  const visit = (node: ts.Node) => {
    if (((ts.isBinaryExpression(node) && [ts.SyntaxKind.PlusToken, ts.SyntaxKind.PlusEqualsToken].includes(node.operatorToken.kind)) || ts.isTemplateExpression(node)) && containsTranslation(node)) {
      failures.push(file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1);
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return failures;
}

/** Callbacks installed on native events are not evaluated during React render. */
export function missingRenderLanguageSubscription(source: string, filename = "screen.tsx") {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const failures: number[] = [];
  const hasHook = (node: ts.Node): boolean => {
    if (ts.isCallExpression(node) && node.expression.getText(file) === "useAppLanguage") return true;
    return ts.forEachChild(node, hasHook) ?? false;
  };
  const containsJsx = (node: ts.Node): boolean => {
    if (ts.isJsxElement(node) || ts.isJsxSelfClosingElement(node) || ts.isJsxFragment(node)) return true;
    return ts.forEachChild(node, containsJsx) ?? false;
  };
  const visit = (node: ts.Node) => {
    if (translationCall(node, file) && /translate(?:App)?$/.test((node as ts.CallExpression).expression.getText(file))) {
      let parent: ts.Node | undefined = node.parent;
      let render = false;
      let subscribed = false;
      let event = false;
      while (parent && !ts.isSourceFile(parent)) {
        if (ts.isJsxAttribute(parent) && /^on[A-Z]/.test(parent.name.getText(file))) event = true;
        if (ts.isJsxExpression(parent) || ts.isJsxElement(parent)) render = true;
        if (ts.isFunctionDeclaration(parent) || ts.isFunctionExpression(parent) || ts.isArrowFunction(parent) || ts.isMethodDeclaration(parent)) {
          // Includes translations assigned to a local variable or memo before JSX.
          if (containsJsx(parent)) render = true;
          if (hasHook(parent)) subscribed = true;
        }
        parent = parent.parent;
      }
      if (render && !event && !subscribed) failures.push(file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return failures;
}
