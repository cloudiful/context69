import { useMemo } from "react";

import { highlightQueryText, highlightRenderedHtml } from "@/utils/search-highlight";
import { renderMarkdownPreviewBlocks } from "@/utils/markdown-preview";

/**
 * Renders one stored block through the shared, escaping markdown renderer and
 * the shared highlighter. Only that renderer's output reaches the DOM: the raw
 * block text never does, so it cannot supply markup, event handlers or links
 * the renderer would refuse.
 */
export function DocumentMarkdown({
  text,
  query,
}: {
  text: string;
  query: string;
}) {
  const rendered = useMemo(() => {
    try {
      const blocks = renderMarkdownPreviewBlocks(text)
        .map((block) => block.html)
        .join("");
      return { html: highlightRenderedHtml(blocks, query), plain: false };
    } catch {
      // A renderer failure must not blank stored text: fall back to the same
      // escaping highlighter the plain-text path uses.
      return { html: highlightQueryText(text, query), plain: true };
    }
  }, [query, text]);

  if (rendered.plain) {
    return (
      <pre
        data-testid="document-block-content"
        className="font-sans text-[0.98rem] leading-8 break-words whitespace-pre-wrap text-foreground [&_mark]:rounded-sm [&_mark]:bg-primary/15 [&_mark]:px-0.5"
        dangerouslySetInnerHTML={{ __html: rendered.html }}
      />
    );
  }

  return (
    <article
      data-testid="document-block-content"
      className="text-[0.98rem] leading-8 break-words text-foreground [&_*:first-child]:mt-0 [&_*:last-child]:mb-0 [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-muted [&_code]:px-1.5 [&_code]:py-0.5 [&_h1]:mt-6 [&_h1]:mb-3 [&_h1]:text-2xl [&_h1]:font-semibold [&_h2]:mt-6 [&_h2]:mb-3 [&_h2]:text-xl [&_h2]:font-semibold [&_h3]:mt-6 [&_h3]:mb-3 [&_h3]:text-lg [&_h3]:font-semibold [&_mark]:rounded-sm [&_mark]:bg-primary/15 [&_mark]:px-0.5 [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-3 [&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:bg-muted/50 [&_pre]:px-4 [&_pre]:py-3 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6"
      dangerouslySetInnerHTML={{ __html: rendered.html }}
    />
  );
}
