import { skipToken, useQuery } from "@tanstack/react-query";
import { ExternalLink, Loader2 } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usePreferences } from "@/hooks/use-preferences";
import { formatDate, formatScore } from "@/lib/format";
import { apiClient } from "@/services/api";
import type { SearchHit } from "@/services/api";

import { matchReasonTokens, stripTitlePrefix } from "@/utils/search";
import { HighlightedText } from "./search-highlight";

const PREVIEW_CHUNK_LIMIT = 20;

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

/**
 * Single-selection result preview. The document is fetched through the
 * generated typed client, so the preview shows the stored document rather than
 * trusting the search hit alone.
 */
export function SearchPreview({
  hit,
  query,
  open,
  onOpenChange,
  onOpen,
}: {
  hit: SearchHit | null;
  query: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpen: (hit: SearchHit) => void;
}) {
  const { t } = useTranslation();
  const { effectiveTimeZone } = usePreferences();
  const documentQuery = useQuery({
    queryKey: ["search-preview-document", hit?.document_id ?? null],
    queryFn: hit ? () => apiClient.getDocument(hit.document_id) : skipToken,
    enabled: !!hit && open,
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const document = documentQuery.data;
  const published = useMemo(
    () => formatDate(hit?.published_at),
    [hit?.published_at, effectiveTimeZone],
  );
  const reasons = matchReasonTokens(hit?.match_reason);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[88vh] w-[min(96vw,72rem)] max-w-[min(96vw,72rem)] flex-col">
        <DialogHeader>
          <DialogTitle className="pr-8">{hit?.title ?? t("search.preview.title")}</DialogTitle>
          <DialogDescription asChild>
            {hit ? (
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                <span>{published}</span>
                <span aria-hidden="true">·</span>
                <span className="min-w-0 truncate">{hit.group_path}</span>
                <span aria-hidden="true">·</span>
                <span
                  data-testid="search-preview-score"
                  title={t("search.result.scoreLabel", { score: formatScore(hit.score) })}
                  className="font-mono tabular-nums"
                >
                  {formatScore(hit.score)}
                </span>
                {reasons.map((reason) => (
                  <Badge key={reason} variant="secondary" className="font-normal">
                    {reasonLabel(reason, t)}
                  </Badge>
                ))}
              </div>
            ) : (
              <span />
            )}
          </DialogDescription>
        </DialogHeader>

        <div data-testid="search-selection-preview" className="grid min-h-0 flex-1 gap-3 overflow-y-auto">
          {hit && /^https?:\/\//i.test(hit.source_uri ?? "") ? (
            <a
              href={hit.source_uri ?? undefined}
              target="_blank"
              rel="noopener noreferrer"
              className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground underline decoration-dotted underline-offset-2 hover:text-foreground"
              title={hit.source_uri ?? undefined}
            >
              <ExternalLink aria-hidden="true" className="size-3.5 shrink-0" />
              <span className="truncate">{hit.source_uri}</span>
            </a>
          ) : null}

          {documentQuery.isPending ? (
            <p role="status" className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
              <Loader2 aria-hidden="true" className="size-4 animate-spin" />
              {t("search.preview.loading")}
            </p>
          ) : documentQuery.isError ? (
            <Alert variant="destructive">
              <AlertTitle>{t("search.preview.loadFailed")}</AlertTitle>
              <AlertDescription className="grid gap-2">
                <span>{documentQuery.error instanceof Error ? documentQuery.error.message : ""}</span>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="w-fit"
                  onClick={() => void documentQuery.refetch()}
                >
                  {t("common.retry")}
                </Button>
              </AlertDescription>
            </Alert>
          ) : document ? (
            <div className="grid gap-3">
              {document.summary ? (
                <section className="grid gap-1">
                  <h3 className="text-xs font-medium uppercase text-muted-foreground">
                    {t("search.preview.summary")}
                  </h3>
                  <p className="text-sm leading-6">
                    <HighlightedText text={stripTitlePrefix(document.summary, document.title)} query={query} />
                  </p>
                </section>
              ) : null}
              <section className="grid gap-2">
                <h3 className="text-xs font-medium uppercase text-muted-foreground">
                  {t("search.preview.content")}
                </h3>
                {document.chunks.slice(0, PREVIEW_CHUNK_LIMIT).map((chunk) => (
                  <p key={chunk.chunk_id} className="whitespace-pre-line text-sm leading-6">
                    <HighlightedText
                      text={stripTitlePrefix(chunk.text, document.title)}
                      query={query}
                    />
                  </p>
                ))}
              </section>
            </div>
          ) : null}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.close")}
          </Button>
          <Button
            type="button"
            data-testid="search-preview-open"
            disabled={!hit}
            onClick={() => {
              if (hit) {
                onOpen(hit);
              }
            }}
          >
            {t("common.open")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
