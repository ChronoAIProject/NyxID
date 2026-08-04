import { Link } from "@tanstack/react-router";

export function ArticleNotFound() {
  return (
    <div className="px-6 pt-20 pb-24">
      <div className="border-landing-border-subtle bg-landing-surface mx-auto max-w-md rounded-2xl border p-10 text-center">
        <p className="font-mono text-xs tracking-widest text-primary uppercase">
          Not found
        </p>
        <p className="mt-3 font-serif text-2xl text-white">
          This dispatch isn't here.
        </p>
        <p className="mt-3 text-sm text-gray-400">
          It may have been unpublished or the URL is wrong.
        </p>
        <Link
          to="/blog"
          className="mt-6 inline-flex rounded-lg border border-primary/40 px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary/10"
        >
          Back to Field Notes
        </Link>
      </div>
    </div>
  );
}
