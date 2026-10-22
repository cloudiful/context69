import type { ReactNode } from "react";

import { findSearchHighlightRanges, searchHighlightTerms } from "@/utils/search-highlight";

/**
 * Renders `text` as React nodes with every query term wrapped in a `<mark>`.
 * The source text never passes through HTML, so fetched titles and snippets
 * cannot inject markup.
 */
export function HighlightedText({ text, query }: { text: string; query: string }) {
  const ranges = findSearchHighlightRanges(text, searchHighlightTerms(query));
  if (ranges.length === 0) {
    return <>{text}</>;
  }

  const nodes: ReactNode[] = [];
  let cursor = 0;

  ranges.forEach((range, index) => {
    if (range.start > cursor) {
      nodes.push(text.slice(cursor, range.start));
    }
    nodes.push(
      <mark
        key={`mark-${index}`}
        className="rounded-sm bg-primary/15 px-0.5 text-inherit"
      >
        {text.slice(range.start, range.end)}
      </mark>,
    );
    cursor = range.end;
  });

  if (cursor < text.length) {
    nodes.push(text.slice(cursor));
  }

  return <>{nodes}</>;
}
