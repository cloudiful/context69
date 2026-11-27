import { useQueries } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { apiClient } from "@/services/api";
import type { LibraryFileDetailResponse, SearchHit } from "@/services/api";

import { DocumentMarkdown } from "@/features/documents/document-markdown";

import {
  libraryPreviewContent,
  libraryPreviewCurrentPage,
  libraryPreviewDocumentState,
  libraryPreviewPage,
  libraryPreviewPageCount,
  libraryPreviewPageState,
  libraryPreviewReadiness,
  type LibraryPreviewDetailState,
} from "./library-preview-content";
import type { MoveDialogState, SourceFolderState } from "./use-library-workspace";
import { SourceConfigEditor } from "./source-config-editor";

export function CreateFolderDialog({
  open,
  busy,
  parentName,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  parentName: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");

  useEffect(() => {
    if (open) {
      setName("");
    }
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("library.newFolder")}</DialogTitle>
          <DialogDescription>{parentName}</DialogDescription>
        </DialogHeader>
        <FormField label={t("library.folderName")} htmlFor="new-folder-name">
          <Input
            id="new-folder-name"
            value={name}
            autoFocus
            data-testid="new-folder-name"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && name.trim()) {
                onSubmit(name);
              }
            }}
          />
        </FormField>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !name.trim()}
            aria-busy={busy}
            onClick={() => onSubmit(name)}
          >
            {t("common.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function CreateTextFileDialog({
  open,
  busy,
  parentName,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  parentName: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: { title: string; content: string }) => void;
}) {
  const { t } = useTranslation();
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");

  useEffect(() => {
    if (open) {
      setTitle("");
      setContent("");
    }
  }, [open]);

  const valid = title.trim().length > 0 && content.trim().length > 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("library.newTextFile")}</DialogTitle>
          <DialogDescription>{parentName}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <FormField label={t("library.textTitle")} htmlFor="new-text-title">
            <Input
              id="new-text-title"
              value={title}
              autoFocus
              data-testid="new-text-title"
              onChange={(event) => setTitle(event.target.value)}
            />
          </FormField>
          <FormField label={t("library.textContent")} htmlFor="new-text-content">
            <Textarea
              id="new-text-content"
              rows={10}
              value={content}
              data-testid="new-text-content"
              onChange={(event) => setContent(event.target.value)}
            />
          </FormField>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !valid}
            aria-busy={busy}
            onClick={() => onSubmit({ title, content })}
          >
            {t("common.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function MoveEntryDialog({
  state,
  busy,
  onOpenChange,
  onSubmit,
}: {
  state: MoveDialogState | null;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (targetFolderId: string | null) => void;
}) {
  const { t } = useTranslation();
  const [target, setTarget] = useState<string>("__root__");

  useEffect(() => {
    if (state) {
      setTarget(state.entry.parentFolderId ?? "__root__");
    }
  }, [state]);

  return (
    <Dialog open={!!state} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("common.move")}</DialogTitle>
          <DialogDescription>{state?.entry.name}</DialogDescription>
        </DialogHeader>
        <FormField label={t("library.targetFolder")} htmlFor="move-target-folder">
          <Select value={target} onValueChange={setTarget}>
            <SelectTrigger id="move-target-folder" className="w-full" data-testid="move-target-folder">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__root__">{t("library.rootFolder")}</SelectItem>
              {state?.options.map((option) => (
                <SelectItem key={option.value ?? "__root__"} value={option.value ?? "__root__"}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy}
            aria-busy={busy}
            onClick={() => onSubmit(target === "__root__" ? null : target)}
          >
            {t("common.move")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SourceFolderDialog({
  state,
  onOpenChange,
  onChangeValue,
  onChangeFolderName,
  onSubmit,
}: {
  state: SourceFolderState;
  onOpenChange: (open: boolean) => void;
  onChangeValue: (value: string) => void;
  onChangeFolderName: (value: string) => void;
  onSubmit: () => void;
}) {
  const { t } = useTranslation();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!state.open) {
      setError(null);
    }
  }, [state.open]);

  function submit() {
    try {
      const parsed: unknown = JSON.parse(state.value);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        setError(t("library.sourceConfigInvalid"));
        return;
      }
      const record = parsed as Record<string, unknown>;
      if (typeof record.source_key !== "string" || !record.source_key.trim()) {
        setError(t("library.sourceConfigMissingKey"));
        return;
      }
      setError(null);
      onSubmit();
    } catch {
      setError(t("library.sourceConfigInvalid"));
    }
  }

  return (
    <Dialog open={state.open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,60rem)] sm:max-w-[min(96vw,60rem)]">
        <DialogHeader>
          <DialogTitle>{state.title}</DialogTitle>
          <DialogDescription>{t("library.sourceConfigDescription")}</DialogDescription>
        </DialogHeader>
        <div className="grid min-w-0 gap-2">
          <SourceConfigEditor
            value={state.value}
            onChange={(next) => {
              setError(null);
              onChangeValue(next);
            }}
            disabled={state.busy}
            showFolderName={!state.folderId}
            folderName={state.folderName}
            onFolderNameChange={onChangeFolderName}
          />
          {error ? (
            <p role="alert" className="text-sm text-destructive">{error}</p>
          ) : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={state.busy || (!state.folderId && !state.folderName.trim())}
            aria-busy={state.busy}
            onClick={submit}
          >
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function PreviewDialog({
  open,
  title,
  detail,
  selectedFileId,
  detailSettled,
  detailRefreshing,
  detailFailed,
  loading,
  error,
  canRetry,
  retrying,
  canReprocess,
  reprocessing,
  canRelease,
  releasing,
  onOpenChange,
  onRetry,
  onReprocess,
  onRelease,
  onRetryDetail,
}: {
  open: boolean;
  title: string;
  detail: LibraryFileDetailResponse | null;
  selectedFileId: string | null;
  detailSettled: boolean;
  detailRefreshing: boolean;
  detailFailed: boolean;
  loading: boolean;
  error: unknown;
  canRetry: boolean;
  retrying: boolean;
  canReprocess: boolean;
  reprocessing: boolean;
  canRelease: boolean;
  releasing: boolean;
  onOpenChange: (open: boolean) => void;
  onRetry: () => void;
  onReprocess: () => void;
  onRelease: () => void;
  onRetryDetail: () => void;
}) {
  const { t } = useTranslation();
  const message = error instanceof Error ? error.message : error ? t("library.detailLoadFailed") : null;

  // The document set of a file is only known from its own settled detail.
  // Retained detail of a running refresh or of a failed read is unverified,
  // so it issues no document reads and never presents an old association set
  // as the current one.
  const detailState = useMemo<LibraryPreviewDetailState>(() => ({
    fileId: detail?.file_id ?? null,
    settled: detailSettled,
    refreshing: detailRefreshing,
    failed: detailFailed,
  }), [detail, detailFailed, detailRefreshing, detailSettled]);

  const readiness = useMemo(
    () => libraryPreviewReadiness(selectedFileId, detailState, detail?.sections ?? []),
    [detail, detailState, selectedFileId],
  );

  const documentQueries = useQueries({
    queries: readiness.refs.map((ref) => ({
      queryKey: ["document", ref.documentId],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        apiClient.getDocument(ref.documentId, { signal }),
      enabled: readiness.ready,
      retry: false,
    })),
  });

  const documentStates = useMemo(() => (
    readiness.ready
      ? readiness.refs.map((ref, index) => libraryPreviewDocumentState(ref, documentQueries[index]))
      : []
  ), [documentQueries, readiness]);

  const content = useMemo(() => libraryPreviewContent(documentStates), [documentStates]);

  // The page belongs to one file. The identity is the selected file, not the
  // transient detail data: a refresh of the same file keeps its page, while
  // another file starts at page one in the same render, so the previous
  // file's page is never shown with the new file's content. A closed preview
  // has no selected file, so it clears the reading scope and every fresh
  // open starts at page one.
  const [pageState, setPageState] = useState({ scope: "", page: 1 });
  const scope = selectedFileId ?? "";
  const resolvedPageState = libraryPreviewPageState(scope, pageState);
  if (resolvedPageState !== pageState) {
    setPageState(resolvedPageState);
  }
  const totalPages = libraryPreviewPageCount(content.blocks);
  const currentPage = libraryPreviewCurrentPage(resolvedPageState.page, totalPages);
  const visibleBlocks = useMemo(
    () => libraryPreviewPage(content.blocks, currentPage),
    [content.blocks, currentPage],
  );

  const goToPage = useCallback((page: number) => {
    setPageState({ scope, page });
  }, [scope]);

  const retryDocuments = useCallback(() => {
    for (const query of documentQueries) {
      if (query.status === "error") {
        void query.refetch();
      }
    }
  }, [documentQueries]);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [currentPage, scope]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,64rem)] sm:max-w-[min(96vw,64rem)]">
        <DialogHeader>
          <DialogTitle className="truncate">{title}</DialogTitle>
          <DialogDescription>
            {detail ? `${detail.group_path}/${detail.folder_path}` : t("common.loading")}
          </DialogDescription>
        </DialogHeader>
        <div
          className="grid max-h-[70vh] gap-3 overflow-y-auto"
          data-testid="library-preview"
          ref={scrollRef}
        >
          {loading ? <p className="text-sm text-muted-foreground">{t("common.loading")}</p> : null}
          {message ? <p role="alert" className="text-sm text-destructive">{message}</p> : null}
          {detail ? (
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span>{detail.media_type}</span>
              {detail.source_available === false ? (
                <span className="text-destructive">{t("library.sourceMissing")}</span>
              ) : null}
            </div>
          ) : null}
          {!readiness.ready ? (
            readiness.failed ? (
              <div
                role="alert"
                className="grid gap-2 rounded-md border border-destructive/40 p-3 text-sm"
                data-testid="library-preview-detail-failed"
              >
                <p className="text-destructive">{t("library.previewDetailFailed")}</p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="w-fit"
                  onClick={onRetryDetail}
                >
                  {t("common.retry")}
                </Button>
              </div>
            ) : readiness.refreshing ? (
              <p
                className="text-sm text-muted-foreground"
                data-testid="library-preview-refreshing"
              >
                {t("library.previewContentRefreshing")}
              </p>
            ) : null
          ) : (
            <>
              {content.pending.length > 0 ? (
                <p className="text-sm text-muted-foreground" data-testid="library-preview-loading">
                  {t("library.previewContentLoading")}
                </p>
              ) : null}
              {content.refreshing.length > 0 ? (
                <p
                  className="text-sm text-muted-foreground"
                  data-testid="library-preview-content-refreshing"
                >
                  {t("library.previewContentRefreshing")}
                </p>
              ) : null}
              {content.failed.length > 0 ? (
                <div
                  role="alert"
                  className="grid gap-2 rounded-md border border-destructive/40 p-3 text-sm"
                  data-testid="library-preview-incomplete"
                >
                  <p className="text-destructive">{t("library.previewContentIncomplete")}</p>
                  <ul className="grid gap-1 text-xs text-muted-foreground">
                    {content.failed.map((state) => (
                      <li key={state.documentId}>
                        {t("library.previewDocumentFailed", {
                          title: state.title || t("library.previewSection"),
                        })}
                      </li>
                    ))}
                  </ul>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="w-fit"
                    onClick={retryDocuments}
                  >
                    {t("common.retry")}
                  </Button>
                </div>
              ) : null}
              {content.complete && content.blocks.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("library.previewEmpty")}</p>
              ) : null}
              {visibleBlocks.map((entry) => (
                <section
                  key={`${entry.documentId}-${entry.block.chunk.chunk_id}`}
                  className="grid gap-1 rounded-md border p-3"
                >
                  {entry.sectionTitle ? (
                    <h3 className="text-sm font-medium">{entry.sectionTitle}</h3>
                  ) : null}
                  {entry.block.valid ? (
                    <DocumentMarkdown text={entry.block.chunk.text} query="" />
                  ) : (
                    <p className="text-sm text-destructive">
                      {entry.block.reason === "empty"
                        ? t("library.previewBlockEmpty")
                        : t("library.previewBlockChanged")}
                    </p>
                  )}
                </section>
              ))}
              {totalPages > 1 ? (
                <div className="flex flex-wrap items-center justify-end gap-2">
                  <span className="text-xs text-muted-foreground" data-testid="library-preview-page-status">
                    {t("common.pageOf", { page: currentPage, totalPages })}
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    aria-label={t("common.previous")}
                    disabled={currentPage <= 1}
                    onClick={() => goToPage(currentPage - 1)}
                  >
                    {t("common.previous")}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    aria-label={t("common.next")}
                    disabled={currentPage >= totalPages}
                    onClick={() => goToPage(currentPage + 1)}
                  >
                    {t("common.next")}
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </div>
        <DialogFooter>
          {canRetry ? (
            <Button type="button" variant="outline" disabled={retrying} aria-busy={retrying} onClick={onRetry}>
              {t("common.retry")}
            </Button>
          ) : null}
          {canReprocess ? (
            <Button
              type="button"
              variant="outline"
              disabled={reprocessing}
              aria-busy={reprocessing}
              onClick={onReprocess}
            >
              {t("library.reprocess")}
            </Button>
          ) : null}
          {canRelease ? (
            <Button type="button" variant="outline" disabled={releasing} aria-busy={releasing} onClick={onRelease}>
              {t("library.releaseSource")}
            </Button>
          ) : null}
          <Button type="button" onClick={() => onOpenChange(false)}>{t("common.close")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ScopedSearchDialog({
  open,
  query,
  folderName,
  loading,
  results,
  onOpenChange,
  onOpenResult,
}: {
  open: boolean;
  query: string;
  folderName: string;
  loading: boolean;
  results: SearchHit[];
  onOpenChange: (open: boolean) => void;
  onOpenResult: (hit: SearchHit) => void;
}) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,48rem)] sm:max-w-[min(96vw,48rem)]">
        <DialogHeader>
          <DialogTitle>{t("search.scoped.title", { folder: folderName })}</DialogTitle>
          <DialogDescription>{t("search.scoped.description", { query })}</DialogDescription>
        </DialogHeader>
        <div className="grid max-h-[60vh] gap-2 overflow-y-auto" data-testid="scoped-search-results">
          {loading ? <p className="text-sm text-muted-foreground">{t("common.loading")}</p> : null}
          {!loading && results.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("search.scoped.empty")}</p>
          ) : null}
          {results.map((hit) => (
            <button
              key={hit.chunk_id}
              type="button"
              className="grid gap-1 rounded-md border p-3 text-left hover:bg-accent"
              onClick={() => onOpenResult(hit)}
            >
              <span className="text-sm font-medium">{hit.title}</span>
              <span className="text-xs text-muted-foreground">{hit.library_path ?? hit.group_path}</span>
              <span className="line-clamp-3 text-sm">{hit.chunk_text}</span>
            </button>
          ))}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
