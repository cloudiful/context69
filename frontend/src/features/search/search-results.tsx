import { ExternalLink, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { usePreferences } from "@/hooks/use-preferences";
import { formatDate, formatScore } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { SearchHit, SearchPagination } from "@/services/api";

import {
  buildSnippet,
  matchReasonTokens,
  stripTitlePrefix,
} from "@/utils/search";
import { HighlightedText } from "./search-highlight";

const PAGE_SIZE_OPTIONS = [8, 16, 24, 50];

function reasonLabel(token: string, t: (key: string) => string): string {
  switch (token) {
    case "semantic":
      return t("search.result.reasonSemantic");
    case "title":
      return t("search.result.reasonTitle");
    case "keyword":
      return t("search.result.reasonKeyword");
    default:
      return token;
  }
}

function isHttpUri(value: string | null | undefined): boolean {
  return !!value && /^https?:\/\//i.test(value.trim());
}

function useElapsed(active: boolean): string {
  const [label, setLabel] = useState("0.0s");

  useEffect(() => {
    if (!active) {
      setLabel("0.0s");
      return;
    }
    const start = Date.now();
    setLabel("0.0s");
    const timer = setInterval(() => {
      setLabel(`${((Date.now() - start) / 1000).toFixed(1)}s`);
    }, 200);
    return () => clearInterval(timer);
  }, [active]);

  return label;
}

function ResultRow({
  hit,
  query,
  selected,
  onSelect,
  onOpen,
}: {
  hit: SearchHit;
  query: string;
  selected: boolean;
  onSelect: (hit: SearchHit) => void;
  onOpen: (hit: SearchHit) => void;
}) {
  const { t } = useTranslation();
  const { effectiveTimeZone } = usePreferences();

  const body = stripTitlePrefix(hit.chunk_text ?? "", hit.title ?? "");
  const snippet = useMemo(() => buildSnippet(body, query), [body, query]);
  const published = useMemo(
    () => formatDate(hit.published_at),
    [hit.published_at, effectiveTimeZone],
  );
  const reasons = matchReasonTokens(hit.match_reason);

  return (
    <li
      data-testid="search-result-item"
      data-selected={selected ? "" : undefined}
      className={cn(
        "min-w-0 rounded-md px-2 py-2",
        selected ? "bg-muted/40 ring-1 ring-border" : "",
      )}
    >
      <div className="flex min-w-0 items-start justify-between gap-2">
        <button
          type="button"
          data-testid="search-result-select"
          aria-pressed={selected}
          title={`${t("search.result.scoreLabel", { score: formatScore(hit.score) })}${hit.group_path ? ` · ${hit.group_path}` : ""}`}
          className="line-clamp-2 min-w-0 flex-1 text-left text-sm font-semibold leading-6 hover:underline"
          onClick={() => onSelect(hit)}
        >
          <HighlightedText text={hit.title ?? ""} query={query} />
        </button>
        <div className="flex shrink-0 items-center gap-1">
          <span
            data-testid="search-result-score"
            title={t("search.result.scoreLabel", { score: formatScore(hit.score) })}
            className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs tabular-nums text-muted-foreground"
          >
            {formatScore(hit.score)}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-testid="search-result-preview"
            onClick={() => onSelect(hit)}
          >
            {t("search.result.preview")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-testid="search-result-open"
            onClick={() => onOpen(hit)}
          >
            {t("common.open")}
          </Button>
          {isHttpUri(hit.source_uri) ? (
            <Button asChild variant="ghost" size="icon-sm">
              <a
                data-testid="search-result-source-link"
                href={hit.source_uri}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={t("search.result.openSourceLink")}
                title={hit.source_uri}
              >
                <ExternalLink aria-hidden="true" />
              </a>
            </Button>
          ) : null}
        </div>
      </div>

      {!isHttpUri(hit.source_uri) && hit.is_library_file && hit.library_path ? (
        <p className="mt-1 max-w-full truncate text-xs text-muted-foreground">{hit.library_path}</p>
      ) : null}

      <p className="mt-1 line-clamp-3 min-w-0 break-words text-[0.88rem] leading-6 text-muted-foreground">
        <HighlightedText text={snippet} query={query} />
      </p>

      <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted-foreground">
        <span>{published}</span>
        <span aria-hidden="true">·</span>
        <span className="min-w-0 truncate">{hit.group_path}</span>
        {reasons.map((reason) => (
          <Badge key={reason} variant="secondary" className="font-normal">
            {reasonLabel(reason, t)}
          </Badge>
        ))}
      </div>
    </li>
  );
}

export function SearchResults({
  items,
  pagination,
  query,
  loading,
  refining,
  fallbackNotice,
  searched,
  sort,
  selectedChunkId,
  onSelect,
  onOpen,
  onCancel,
  onPrev,
  onNext,
  onPageSize,
}: {
  items: SearchHit[];
  pagination: SearchPagination | null;
  query: string;
  loading: boolean;
  refining: boolean;
  fallbackNotice: boolean;
  searched: boolean;
  sort: "relevance" | "date";
  selectedChunkId: string | null;
  onSelect: (hit: SearchHit) => void;
  onOpen: (hit: SearchHit) => void;
  onCancel: () => void;
  onPrev: () => void;
  onNext: () => void;
  onPageSize: (size: number) => void;
}) {
  const { t } = useTranslation();
  const elapsed = useElapsed(loading || refining);

  const total = pagination?.total ?? 0;
  const summary = pagination
    ? pagination.total_is_exact === false
      ? t("search.resultsAtLeast", { count: total })
      : `${t("search.workspace.resultsLabel")}: ${total}`
    : "";

  const pageSizeOptions = pagination
    ? PAGE_SIZE_OPTIONS.includes(pagination.page_size)
      ? PAGE_SIZE_OPTIONS
      : [...PAGE_SIZE_OPTIONS, pagination.page_size].sort((left, right) => left - right)
    : PAGE_SIZE_OPTIONS;

  const canGoPrev = !!pagination && sort !== "date" && (pagination.prev_cursor != null || pagination.page > 1);
  const canGoNext = !!pagination && (pagination.next_cursor != null || pagination.has_more === true);

  const showSpinner = loading && !searched;
  const showEmpty = searched && !loading && items.length === 0;

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border">
      <header className="flex min-w-0 flex-wrap items-center justify-between gap-2 border-b px-3 py-2">
        <h2 className="text-base font-semibold">{t("search.resultsTitle")}</h2>
        <div className="flex min-w-0 items-center gap-2">
          {refining ? (
            <span
              data-testid="search-refining"
              className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground"
            >
              <Loader2 aria-hidden="true" className="size-3.5 animate-spin" />
              <span className="truncate">{t("search.streaming.refining")}</span>
              <span className="tabular-nums">{elapsed}</span>
            </span>
          ) : null}
          {pagination ? <span className="text-xs text-muted-foreground">{summary}</span> : null}
        </div>
      </header>

      {fallbackNotice ? (
        <p
          data-testid="search-stream-fallback-notice"
          className="mx-2 mt-2 shrink-0 rounded-md bg-muted px-2.5 py-1.5 text-xs text-muted-foreground"
        >
          {t("search.streaming.fallbackNotice")}
        </p>
      ) : null}

      {showSpinner ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 py-12 text-center">
          <Loader2
            aria-hidden="true"
            data-testid="search-loading"
            className="size-10 animate-spin text-muted-foreground"
          />
          <div className="grid gap-1">
            <p className="text-sm font-medium">{t("search.scanningTitle")} · {elapsed}</p>
            <p className="text-xs text-muted-foreground">{t("search.form.waiting")}</p>
          </div>
          <Button type="button" variant="ghost" size="sm" data-testid="search-cancel" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
        </div>
      ) : showEmpty ? (
        <div className="grid flex-1 place-items-center p-4">
          <div className="grid max-w-md gap-1 text-center">
            <p className="text-sm font-medium">{t("search.noMatchesTitle")}</p>
            <p className="text-sm text-muted-foreground">{t("search.noMatchesMessage")}</p>
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
          <div
            data-testid="search-results-list"
            className="min-h-[220px] flex-1 overflow-y-auto overscroll-contain"
          >
            <ul data-testid="search-results-scroll" className="grid min-w-0 gap-1">
              {items.map((hit) => (
                <ResultRow
                  key={hit.chunk_id}
                  hit={hit}
                  query={query}
                  selected={selectedChunkId !== null && selectedChunkId === hit.chunk_id}
                  onSelect={onSelect}
                  onOpen={onOpen}
                />
              ))}
            </ul>
          </div>

          {pagination ? (
            <div className="flex min-h-9 shrink-0 flex-wrap items-center justify-between gap-2 border-t pt-2">
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">{t("common.itemsPerPage")}</span>
                <Select
                  value={String(pagination.page_size)}
                  onValueChange={(value) => onPageSize(Number(value))}
                >
                  <SelectTrigger
                    size="sm"
                    className="w-20"
                    data-testid="search-page-size"
                    aria-label={t("common.itemsPerPage")}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {pageSizeOptions.map((size) => (
                      <SelectItem key={size} value={String(size)}>
                        {size}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="flex min-w-0 items-center gap-1.5">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  data-testid="search-prev"
                  disabled={!canGoPrev}
                  title={sort === "date" ? t("search.pagination.previousDisabledHint") : t("search.pagination.previous")}
                  aria-label={t("search.pagination.previous")}
                  onClick={onPrev}
                >
                  {t("search.pagination.previous")}
                </Button>
                <span className="px-1 text-xs tabular-nums text-muted-foreground">
                  {t("search.pagination.page", { page: pagination.page })}
                </span>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  data-testid="search-next"
                  disabled={!canGoNext}
                  aria-label={t("search.pagination.next")}
                  onClick={onNext}
                >
                  {t("search.pagination.next")}
                </Button>
              </div>
            </div>
          ) : null}
        </div>
      )}
    </section>
  );
}
