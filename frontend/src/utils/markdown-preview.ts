function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function escapeAttribute(value: string): string {
  return escapeHtml(value).replaceAll("`", "&#96;");
}

function sanitizeUrl(value: string): string | null {
  const trimmed = value.trim();

  if (!trimmed) {
    return null;
  }

  if (/^(https?:|mailto:)/i.test(trimmed)) {
    return trimmed;
  }

  return null;
}

function applyInlineMarkdown(value: string): string {
  const codeTokens: string[] = [];
  const linkTokens: string[] = [];

  let rendered = escapeHtml(value);

  rendered = rendered.replace(/`([^`]+)`/g, (_, code: string) => {
    const token = `@@CODE_${codeTokens.length}@@`;
    codeTokens.push(`<code>${code}</code>`);
    return token;
  });

  rendered = rendered.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label: string, href: string) => {
    const safeHref = sanitizeUrl(href);
    if (!safeHref) {
      return label;
    }

    const token = `@@LINK_${linkTokens.length}@@`;
    linkTokens.push(
      `<a href="${escapeAttribute(safeHref)}" target="_blank" rel="noreferrer noopener">${label}</a>`,
    );
    return token;
  });

  rendered = rendered
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_]+)__/g, "<strong>$1</strong>")
    .replace(/(^|[^\*])\*([^*]+)\*/g, "$1<em>$2</em>")
    .replace(/(^|[^_])_([^_]+)_/g, "$1<em>$2</em>");

  rendered = rendered.replaceAll("\n", "<br />");

  for (const [index, html] of codeTokens.entries()) {
    rendered = rendered.replaceAll(`@@CODE_${index}@@`, html);
  }

  for (const [index, html] of linkTokens.entries()) {
    rendered = rendered.replaceAll(`@@LINK_${index}@@`, html);
  }

  return rendered;
}

function isUnorderedListItem(line: string): boolean {
  return /^[-*]\s+/.test(line.trim());
}

function isOrderedListItem(line: string): boolean {
  return /^\d+\.\s+/.test(line.trim());
}

export function renderMarkdownPreview(markdown: string): string {
  const blocks: string[] = [];
  const lines = markdown.replaceAll("\r\n", "\n").replaceAll("\r", "\n").split("\n");

  for (let index = 0; index < lines.length; ) {
    const line = lines[index];
    const trimmed = line.trim();

    if (!trimmed) {
      index += 1;
      continue;
    }

    if (trimmed.startsWith("```")) {
      const codeLines: string[] = [];
      index += 1;

      while (index < lines.length && !lines[index].trim().startsWith("```")) {
        codeLines.push(lines[index]);
        index += 1;
      }

      if (index < lines.length) {
        index += 1;
      }

      blocks.push(`<pre class="library-markdown-pre"><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
      continue;
    }

    const headingMatch = trimmed.match(/^(#{1,6})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      blocks.push(`<h${level}>${applyInlineMarkdown(headingMatch[2])}</h${level}>`);
      index += 1;
      continue;
    }

    if (isUnorderedListItem(line) || isOrderedListItem(line)) {
      const ordered = isOrderedListItem(line);
      const items: string[] = [];

      while (index < lines.length) {
        const candidate = lines[index];
        const matchesList = ordered ? isOrderedListItem(candidate) : isUnorderedListItem(candidate);
        if (!matchesList) {
          break;
        }

        items.push(candidate.trim().replace(ordered ? /^\d+\.\s+/ : /^[-*]\s+/, ""));
        index += 1;
      }

      const tagName = ordered ? "ol" : "ul";
      const itemsHtml = items.map((item) => `<li>${applyInlineMarkdown(item)}</li>`).join("");
      blocks.push(`<${tagName}>${itemsHtml}</${tagName}>`);
      continue;
    }

    const paragraphLines: string[] = [];
    while (index < lines.length) {
      const candidate = lines[index];
      const candidateTrimmed = candidate.trim();

      if (
        !candidateTrimmed
        || candidateTrimmed.startsWith("```")
        || /^(#{1,6})\s+/.test(candidateTrimmed)
        || isUnorderedListItem(candidate)
        || isOrderedListItem(candidate)
      ) {
        break;
      }

      paragraphLines.push(candidateTrimmed);
      index += 1;
    }

    blocks.push(`<p>${applyInlineMarkdown(paragraphLines.join("\n"))}</p>`);
  }

  return blocks.join("");
}
