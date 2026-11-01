import { useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

import type { DocumentResultResponse } from "@/services/api";

import { ResultPayload } from "./result-payload";
import type { DocumentResultProducerGroup, HistoryResult } from "./use-document-results";
import { useDocumentResults } from "./use-document-results";

function CurrentResultGroup({ group }: { group: DocumentResultProducerGroup }) {
  const { t } = useTranslation();
  return (
    <section className="grid gap-2" data-testid="document-result-group">
      <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {group.producer}
      </h3>
      {group.kinds.map((kind) => (
        <div key={kind.resultKind} className="grid gap-1.5" data-testid="document-result-kind">
          <div className="flex flex-wrap items-center gap-1.5">
            <Badge variant="outline">{kind.resultKind}</Badge>
            <span className="text-xs text-muted-foreground">{kind.items[0]?.schema_ref}</span>
          </div>
          {kind.items.map((item) => (
            <ResultPayload key={item.id} result={item.result} />
          ))}
        </div>
      ))}
    </section>
  );
}

function HistoryEntry({ entry }: { entry: HistoryResult }) {
  const { t } = useTranslation();
  return (
    <div className="grid gap-1.5 border-b py-2 last:border-b-0" data-testid="document-result-history-item">
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant={entry.isCurrent ? "default" : "secondary"} data-testid="document-result-version-badge">
          {entry.isCurrent ? t("document.resultCurrent") : t("document.resultHistorical")}
        </Badge>
        <span className="text-sm font-medium">{entry.result.producer}</span>
        <span className="text-xs text-muted-foreground">{entry.result.result_kind}</span>
        <span className="text-xs text-muted-foreground">
          {t("document.resultSourceVersion")}: {entry.result.source_record_hash}
        </span>
      </div>
      <ResultPayload result={entry.result.result} />
    </div>
  );
}

/**
 * Read-only results for one document. The payload stays opaque text; the panel
 * never renders it as markup and never presents result history as document
 * versions.
 */
export function DocumentResultsPanel({
  documentId,
  groupPath,
}: {
  documentId: number;
  groupPath: string;
}) {
  const { t } = useTranslation();
  const results = useDocumentResults(documentId, groupPath);
  const view = results.view;

  return (
    <Card data-testid="document-results">
      <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-2">
        <CardTitle className="text-sm">{t("document.results")}</CardTitle>
        <div className="flex items-center gap-1" role="group" aria-label={t("document.resultsView")}>
          <Button
            type="button"
            size="sm"
            variant={view === "current" ? "secondary" : "ghost"}
            aria-pressed={view === "current"}
            data-testid="document-results-current"
            onClick={() => results.setView("current")}
          >
            {t("document.resultsCurrent")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant={view === "history" ? "secondary" : "ghost"}
            aria-pressed={view === "history"}
            data-testid="document-results-history"
            onClick={() => results.setView("history")}
          >
            {t("document.resultsHistory")}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="grid gap-3">
        {view === "current" ? (
          results.current.error ? (
            <Alert variant="destructive" data-testid="document-results-error">
              <AlertTitle>{t("document.resultsLoadFailed")}</AlertTitle>
              <AlertDescription className="grid gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="w-fit"
                  disabled={results.current.isFetching}
                  onClick={results.current.reload}
                >
                  {t("common.retry")}
                </Button>
              </AlertDescription>
            </Alert>
          ) : results.current.isLoading ? (
            <p className="text-sm text-muted-foreground" role="status" data-testid="document-results-loading">
              {t("document.resultsLoading")}
            </p>
          ) : results.current.groups.length === 0 ? (
            <p className="text-sm text-muted-foreground" data-testid="document-results-empty">
              {t("document.resultsEmptyCurrent")}
            </p>
          ) : (
            <div className="grid gap-3">
              {results.current.groups.map((group) => (
                <CurrentResultGroup key={group.producer} group={group} />
              ))}
              {results.current.hasNextPage ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="w-fit"
                  disabled={results.current.isFetchingNextPage}
                  data-testid="document-results-load-more"
                  onClick={results.current.loadMore}
                >
                  {t("document.resultsLoadMore")}
                </Button>
              ) : null}
            </div>
          )
        ) : results.history.error ? (
          <Alert variant="destructive" data-testid="document-results-error">
            <AlertTitle>{t("document.resultsLoadFailed")}</AlertTitle>
            <AlertDescription className="grid gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="w-fit"
                disabled={results.history.isFetching}
                onClick={results.history.reload}
              >
                {t("common.retry")}
              </Button>
            </AlertDescription>
          </Alert>
        ) : results.history.isLoading ? (
          <p className="text-sm text-muted-foreground" role="status" data-testid="document-results-loading">
            {t("document.resultsLoading")}
          </p>
        ) : results.history.items.length === 0 ? (
          <p className="text-sm text-muted-foreground" data-testid="document-results-empty">
            {t("document.resultsEmptyHistory")}
          </p>
        ) : (
          <div className="grid gap-1">
            {results.history.items.map((entry) => (
              <HistoryEntry key={entry.result.id} entry={entry} />
            ))}
            {results.history.hasNextPage ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="w-fit"
                disabled={results.history.isFetchingNextPage}
                data-testid="document-results-load-more"
                onClick={results.history.loadMore}
              >
                {t("document.resultsLoadMore")}
              </Button>
            ) : null}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
