/** Split one locale-owned rich message without coercing user data or styled links. */
export function translationNodes<T>(message: string, values: Record<string, T>): Array<string | T> {
  return message.split(/(\{\{\w+\}\})/g).map((part) => {
    const name = /^\{\{(\w+)\}\}$/.exec(part)?.[1];
    return name && Object.hasOwn(values, name) ? values[name]! : part;
  });
}
