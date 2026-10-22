import { ArrowLeft, ExternalLink, FileWarning, Library } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { usePreferences } from "@/hooks/use-preferences";
import { formatDate, formatJson, formatTimestamp } from "@/lib/format";
import { visibilityLabel } from "@/lib/visibility-label";
import type { DocumentResponse } from "@/services/api";

import { HighlightedText } from "@/features/search/search-highlight";

import { DocumentMarkdown } from "./document-markdown";
import {
  DOCUMENT_PAGE_SIZE,
  documentBlocks,
  documentPage,
  documentPageCount,
  type DocumentBlock,
} from "./document-model";
import { useDocument } from "./use-document";

function DetailRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="grid grid-cols-[minmax(0,10rem)_minmax(0,1fr)] gap-2 border-b py-1.5 last:border-b-0">
      <dt className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{label}</dt>
      <dd className={mono ? "min-w-0 break-all font-mono text-xs" : "min-w-0 break-words text-sm"}>{value}</dd>
    </div>
  );
}

function ProvenancePanel({
  document,
}: {
  document: DocumentResponse;
}) {
  const { t } = useTranslation();
  const metadata = document.metadata_json ?? {};

  return (
    <Card data-testid="document-provenance">
      <details>
        <summary className="cursor-pointer list-none px-4 py-3 text-sm font-medium select-none">
          {t("document.provenance")}
        </summary>
        <CardContent className="grid gap-1 pt-0">
          <dl>
            <DetailRow label={t("document.groupPath")} value={document.group_path} />
            <DetailRow label={t("groups.groupKey")} value={document.group_key} />
            <DetailRow label={t("document.externalId")} value={document.external_id} />
            <DetailRow label={t("document.sourceKey")} value={document.source_key} />
            <DetailRow label={t("groups.visibility")} value={visibilityLabel(t, document.visibility)} />
            <DetailRow label={t("document.recordHash")} value={document.record_hash} mono />
            {document.library_path ? (
              <DetailRow label={t("document.libraryPath")} value={document.library_path} />
            ) : null}
            {document.library_section_label ? (
              <DetailRow label={t("document.librarySection")} value={document.library_section_label} />
            ) : null}
            <DetailRow label={t("document.updated")} value={formatTimestamp(document.updated_at)} />
            {document.content_locale ? (
              <DetailRow label={t("document.contentLocale")} value={document.content_locale} />
            ) : null}
            {document.requested_locale ? (
              <DetailRow label={t("document.requestedLocale")} value={document.requested_locale} />
            ) : null}
          </dl>
          {Object.keys(metadata).length > 0 ? (
            <div className="grid gap-1 pt-2">
              <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                {t("document.metadata")}
              </p>
              <pre className="max-h-64 overflow-auto rounded-lg bg-muted/50 px-3 py-2 text-xs leading-6 break-words whitespace-pre-wrap">
                {formatJson(metadata)}
              </pre>
            </div>
          ) : null}
        </CardContent>
      </details>
    </Card>
  );
}

function InvalidBlockCard({
  block,
  onReload,
  reloading,
}: {
  block: DocumentBlock;
  onReload: () => void;
  reloading: boolean;
}) {
  const { t } = useTranslation();
  const message = block.reason === "empty"
    ? t("document.emptyBlockMessage")
    : t("document.changedBlockMessage");

  return (
    <Card data-testid="document-invalid-block" className="border-destructive/40">
      <CardContent className="flex flex-wrap items-start gap-3">
        <FileWarning aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />
        <div className="grid min-w-0 flex-1 gap-1">
          <p className="text-sm font-medium">
            {t("document.chunkLabel", { index: block.chunk.chunk_index })}
          </p>
          <p className="text-sm text-muted-foreground">{message}</p>
        </div>
        <Button type="button" variant="outline" size="sm" disabled={reloading} onClick={onReload}>
          {t("common.refresh")}
        </Button>
      </CardContent>
    </Card>
  );
}

/**
 * The document reader: one numeric document id, its stored blocks, and the
 * provenance the API already exposes. Documents are read-only here; mutation
 * actions do not exist in this product surface.
 */
export function DocumentPage() {
  const { t } = useTranslation();
  const { effectiveTimeZone } = usePreferences();
  const state = useDocument();
  const [page, setPage] = useState(1);

  useEffect(() => {
    setPage(1);
  }, [state.documentId]);

  const blocks = useMemo(
    () => (state.document ? documentBlocks(state.document) : []),
    [state.document],
  );
  const pageCount = documentPageCount(blocks);
  const currentPage = Math.min(page, pageCount);
  const visibleBlocks = useMemo(() => documentPage(blocks, currentPage), [blocks, currentPage]);

  const published = useMemo(
    () => (state.document ? formatDate(state.document.published_at) : ""),
    [effectiveTimeZone, state.document],
  );

  const errorMessage = (() => {
    if (!state.error) {
      return null;
    }
    switch (state.errorKind) {
      case "not_found":
        return t("document.notFoundMessage");
      case "forbidden":
        return t("document.forbiddenMessage");
      default:
        return t("document.loadFailedMessage");
    }
  })();

  const errorTitle = state.errorKind === "not_found"
    ? t("document.notFoundTitle")
    : state.errorKind === "forbidden"
      ? t("document.forbiddenTitle")
      : t("document.loadFailed");

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col gap-3 overflow-y-auto" data-testid="document-page">
      <header className="flex min-w-0 flex-wrap items-start justify-between gap-2">
        <div className="grid min-w-0 gap-1">
          <h1 className="truncate text-lg font-semibold">{state.document?.title ?? t("document.title")}</h1>
          {state.document ? (
            <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
              <span>{published}</span>
              <Badge variant="outline" data-testid="document-visibility">
                {visibilityLabel(t, state.document.visibility)}
              </Badge>
              {state.document.content_locale ? (
                <Badge variant="secondary" data-testid="document-content-locale">
                  {t("document.localeBadge", { locale: state.document.content_locale })}
                </Badge>
              ) : null}
              {state.document.requested_locale
                && state.document.requested_locale !== state.document.content_locale ? (
                  <Badge variant="secondary" data-testid="document-requested-locale">
                    {t("document.requestedBadge", { locale: state.document.requested_locale })}
                  </Badge>
                ) : null}
              {state.document.translation_status ? (
                <Badge variant="secondary" data-testid="document-translation-status">
                  {t(`document.translationStatus.${state.document.translation_status}`)}
                </Badge>
              ) : null}
              {state.document.is_fallback ? (
                <Badge variant="outline" data-testid="document-fallback-badge">
                  {t("document.fallbackBadge")}
                </Badge>
              ) : null}
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {state.fromSearch ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              data-testid="document-back-to-search"
              onClick={state.goBackToSearch}
            >
              <ArrowLeft aria-hidden="true" />
              {t("document.backToSearch")}
            </Button>
          ) : null}
          {state.document ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              data-testid="document-open-source"
              disabled={!state.libraryTarget && !state.originUrl}
              onClick={state.openSource}
            >
              {state.libraryTarget ? <Library aria-hidden="true" /> : <ExternalLink aria-hidden="true" />}
              {state.libraryTarget ? t("document.openLibraryFile") : t("document.openOrigin")}
            </Button>
          ) : null}
        </div>
      </header>

      {state.documentId === null ? (
        <Alert variant="destructive" data-testid="document-invalid-id">
          <AlertTitle>{t("document.invalidId")}</AlertTitle>
          <AlertDescription>{t("document.invalidIdMessage")}</AlertDescription>
        </Alert>
      ) : state.isLoading ? (
        <div className="grid gap-2" data-testid="document-loading">
          <p className="text-sm text-muted-foreground">{t("document.loadingMessage")}</p>
          {[0, 1, 2].map((index) => (
            <Card key={index}>
              <CardContent>
                <div className="h-4 w-2/3 animate-pulse rounded bg-muted" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : state.error ? (
        <Alert variant="destructive" data-testid="document-error">
          <AlertTitle>{errorTitle}</AlertTitle>
          <AlertDescription className="grid gap-2">
            <span>{errorMessage}</span>
            {state.errorKind === "load_failed" ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="w-fit"
                disabled={state.isFetching}
                onClick={state.reload}
              >
                {t("common.retry")}
              </Button>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : state.document ? (
        <div className="grid min-w-0 gap-3">
          {state.document.summary ? (
            <Card data-testid="document-summary">
              <CardHeader>
                <CardTitle className="text-sm">{t("document.summary")}</CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-sm leading-7 text-muted-foreground">
                  <HighlightedText text={state.document.summary} query={state.highlightQuery} />
                </p>
              </CardContent>
            </Card>
          ) : null}

          {blocks.length === 0 ? (
            <Alert data-testid="document-no-blocks">
              <AlertTitle>{t("document.noBodyChunksTitle")}</AlertTitle>
              <AlertDescription>{t("document.noBodyChunksMessage")}</AlertDescription>
            </Alert>
          ) : (
            <div className="grid min-w-0 gap-2">
              {visibleBlocks.map((block) => (
                block.valid ? (
                  <Card key={block.chunk.chunk_id} data-testid="document-block">
                    <CardContent className="grid gap-2">
                      <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                        {t("document.chunkLabel", { index: block.chunk.chunk_index })}
                      </p>
                      <DocumentMarkdown text={block.chunk.text} query={state.highlightQuery} />
                    </CardContent>
                  </Card>
                ) : (
                  <InvalidBlockCard
                    key={block.chunk.chunk_id}
                    block={block}
                    onReload={state.reload}
                    reloading={state.isFetching}
                  />
                )
              ))}
            </div>
          )}

          {blocks.length > DOCUMENT_PAGE_SIZE ? (
            <div className="flex flex-wrap items-center justify-end gap-2">
              <span className="text-xs text-muted-foreground" data-testid="document-page-status">
                {t("common.pageOf", { page: currentPage, totalPages: pageCount })}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                aria-label={t("common.previous")}
                disabled={currentPage <= 1}
                onClick={() => setPage(currentPage - 1)}
              >
                {t("common.previous")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                aria-label={t("common.next")}
                disabled={currentPage >= pageCount}
                onClick={() => setPage(currentPage + 1)}
              >
                {t("common.next")}
              </Button>
            </div>
          ) : null}

          <ProvenancePanel document={state.document} />
        </div>
      ) : null}
    </div>
  );
}
