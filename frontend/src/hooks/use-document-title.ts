import { useEffect } from "react";

const BASE_DOCUMENT_TITLE = "nyxid";

let activeOwner: symbol | null = null;

/**
 * Own the document title for the lifetime of a mounted surface.
 *
 * The owner token prevents a late cleanup from replacing a title installed by
 * a newer surface (or by code outside React). When the current owner leaves,
 * return to the application base title.
 */
export function useDocumentTitle(title: string | null): void {
  useEffect(() => {
    if (!title) return;

    const owner = Symbol("document-title");
    activeOwner = owner;
    document.title = title;

    return () => {
      if (activeOwner === owner && document.title === title) {
        activeOwner = null;
        document.title = BASE_DOCUMENT_TITLE;
      }
    };
  }, [title]);
}
