import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";

// Map every relevant markdown element to landing-styled tailwind classes so
// long-form prose visually fits the rest of the site without a global
// stylesheet.
const COMPONENTS: Components = {
  h1: ({ children }) => (
    <h1 className="mt-12 mb-4 font-serif text-3xl text-white md:text-4xl">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="mt-12 mb-3 font-serif text-2xl text-white md:text-3xl">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="mt-8 mb-2 font-mono text-lg font-medium text-white">
      {children}
    </h3>
  ),
  p: ({ children }) => (
    <p className="mb-5 leading-relaxed text-gray-300">{children}</p>
  ),
  a: ({ href, children }) => (
    <a
      href={href}
      target={href?.startsWith("http") ? "_blank" : undefined}
      rel={href?.startsWith("http") ? "noopener noreferrer" : undefined}
      className="text-primary decoration-primary/40 underline-offset-4 transition-colors hover:text-void-300 hover:decoration-void-300 underline"
    >
      {children}
    </a>
  ),
  strong: ({ children }) => (
    <strong className="font-semibold text-white">{children}</strong>
  ),
  em: ({ children }) => (
    <em className="text-gray-200 italic">{children}</em>
  ),
  blockquote: ({ children }) => (
    <blockquote className="border-primary/60 my-6 border-l-2 pl-5 font-serif text-xl leading-relaxed text-white italic md:text-2xl">
      {children}
    </blockquote>
  ),
  ul: ({ children }) => (
    <ul className="mb-5 list-disc space-y-2 pl-6 leading-relaxed text-gray-300 marker:text-primary">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-5 list-decimal space-y-2 pl-6 leading-relaxed text-gray-300 marker:font-mono marker:text-gray-500">
      {children}
    </ol>
  ),
  li: ({ children }) => <li className="pl-1">{children}</li>,
  hr: () => <hr className="border-landing-border-subtle my-10" />,
  table: ({ children }) => (
    <div className="border-landing-border-subtle bg-landing-surface my-6 overflow-x-auto rounded-xl border">
      <table className="w-full text-left text-sm text-gray-300">
        {children}
      </table>
    </div>
  ),
  thead: ({ children }) => (
    <thead className="border-landing-border-subtle border-b font-mono text-xs text-gray-500 uppercase">
      {children}
    </thead>
  ),
  tr: ({ children }) => (
    <tr className="border-landing-border-subtle border-b last:border-0">
      {children}
    </tr>
  ),
  th: ({ children }) => <th className="px-4 py-2.5">{children}</th>,
  td: ({ children }) => <td className="px-4 py-2.5">{children}</td>,
  img: ({ src, alt }) => (
    <img
      src={src}
      alt={alt ?? ""}
      className="border-landing-border-subtle my-6 w-full rounded-2xl border"
      loading="lazy"
    />
  ),
  // react-markdown wraps code blocks in <pre><code className="language-...">.
  // Inline code arrives without a parent <pre>; we infer by looking at whether
  // a className is present.
  code: ({ className, children }) => {
    const isBlock = typeof className === "string" && className.startsWith("language-");
    if (isBlock) {
      // Block code — let <pre> wrap; we just render the <code> raw inside.
      return <code className={className}>{children}</code>;
    }
    return (
      <code className="bg-primary/10 text-void-300 rounded px-1.5 py-0.5 font-mono text-[0.85em]">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="border-landing-border-subtle bg-landing-surface my-6 overflow-x-auto rounded-xl border p-5 font-mono text-sm leading-relaxed text-gray-200">
      {children}
    </pre>
  ),
};

export function ArticleBody({ markdown }: { readonly markdown: string }) {
  return (
    <div className="text-gray-300">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSanitize]}
        components={COMPONENTS}
      >
        {markdown}
      </ReactMarkdown>
    </div>
  );
}
