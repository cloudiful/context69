export type MarkdownPreviewBlock = {
  html: string;
  weight: number;
};

export function paginateMarkdownPreview(
  blocks: MarkdownPreviewBlock[],
  capacity = 18,
): MarkdownPreviewBlock[][] {
  if (blocks.length === 0) {
    return [];
  }

  const pages: MarkdownPreviewBlock[][] = [];
  let page: MarkdownPreviewBlock[] = [];
  let pageWeight = 0;

  for (const block of blocks) {
    if (page.length > 0 && pageWeight + block.weight > capacity) {
      pages.push(page);
      page = [];
      pageWeight = 0;
    }

    page.push(block);
    pageWeight += block.weight;
  }

  if (page.length > 0) {
    pages.push(page);
  }

  return pages;
}
