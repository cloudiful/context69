import { useForm } from "@tanstack/react-form";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type {
  GitCodeSearchHit,
  GitRepositoryFile,
  GitRepositorySource,
} from "@/services/api";
import { apiClient } from "@/services/api";

import { required } from "../dialogs/fields";
import { GitIndexStatusBadge, gitSkipTotal, shortSha } from "../presentation";
import { gitFilesRefetchInterval } from "../polling";
import { FilesTable } from "../tables/files";

const FILE_PAGE_SIZE = 50;
const MAX_CONTENT_WINDOW_LINES = 400;

type SearchFormValues = {
  query: string;
  pathPrefix: string;
  language: string;
  limit: string;
};

type SearchState = {
  query: string;
  pathPrefix?: string;
  language?: string;
  limit?: number;
};

export function SourceDetail({
  groupPath,
  source,
  canManage,
  onEdit,
  onSync,
  onRebuild,
  onWebhook,
  onDelete,
}: {
  groupPath: string;
  source: GitRepositorySource;
  canManage: boolean;
  onEdit: () => void;
  onSync: () => void;
  onRebuild: () => void;
  onWebhook: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const [cursor, setCursor] = useState<string | undefined>(undefined);
  const [cursorStack, setCursorStack] = useState<Array<string | undefined>>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [contentWindow, setContentWindow] = useState<{ startLine: number; endLine: number } | null>(null);
  const [search, setSearch] = useState<SearchState | null>(null);
  const generationKey = source.active_generation_key ?? "none";

  const filesQuery = useQuery({
    queryKey: ["git-files", groupPath, source.repository_key, generationKey, cursor ?? "root"],
    queryFn: ({ signal }) => apiClient.listGitRepositoryFiles(
      groupPath,
      source.repository_key,
      { limit: FILE_PAGE_SIZE, cursor },
      { signal },
    ),
    retry: false,
    refetchInterval: () => gitFilesRefetchInterval(source.index_status),
  });

  const fileDetailQuery = useQuery({
    queryKey: ["git-file", groupPath, source.repository_key, generationKey, selectedPath],
    queryFn: ({ signal }) => apiClient.getGitRepositoryFile(
      groupPath,
      source.repository_key,
      selectedPath as string,
      { signal },
    ),
    enabled: !!selectedPath,
    retry: false,
  });

  const windowStart = contentWindow?.startLine ?? 1;
  const windowEnd = contentWindow?.endLine ?? Math.min(fileDetailQuery.data?.file.line_count ?? MAX_CONTENT_WINDOW_LINES, MAX_CONTENT_WINDOW_LINES);
  const contentQuery = useInfiniteQuery({
    queryKey: ["git-content", groupPath, source.repository_key, generationKey, selectedPath, windowStart, windowEnd],
    queryFn: ({ pageParam, signal }) => apiClient.getGitRepositoryFileContent(
      groupPath,
      source.repository_key,
      {
        path: selectedPath as string,
        startLine: windowStart,
        endLine: windowEnd,
        cursor: pageParam,
      },
      { signal },
    ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => (
      lastPage.pagination.has_more ? lastPage.pagination.next_cursor ?? undefined : undefined
    ),
    enabled: !!selectedPath,
    retry: false,
  });

  const searchForm = useForm({
    defaultValues: {
      query: "",
      pathPrefix: "",
      language: "",
      limit: "20",
    } satisfies SearchFormValues,
    onSubmit: async ({ value }) => {
      const limit = Number(value.limit);
      setSearch({
        query: value.query.trim(),
        pathPrefix: value.pathPrefix.trim() || undefined,
        language: value.language.trim() || undefined,
        limit: Number.isFinite(limit) && limit >= 1 && limit <= 50 ? limit : 20,
      });
    },
  });

  const searchQuery = useQuery({
    queryKey: ["git-code-search", groupPath, source.repository_key, generationKey, search],
    queryFn: ({ signal }) => apiClient.searchGitRepositoryCode(
      groupPath,
      source.repository_key,
      {
        query: search?.query ?? "",
        pathPrefix: search?.pathPrefix,
        language: search?.language,
        limit: search?.limit,
      },
      { signal },
    ),
    enabled: search !== null && (search?.query.length ?? 0) > 0,
    retry: false,
  });

  function selectFile(file: GitRepositoryFile) {
    setSelectedPath(file.path);
    setContentWindow({
      startLine: 1,
      endLine: Math.max(1, Math.min(file.line_count, MAX_CONTENT_WINDOW_LINES)),
    });
  }

  function selectHit(hit: GitCodeSearchHit) {
    setSelectedPath(hit.path);
    setContentWindow({
      startLine: Math.max(1, hit.start_line - 10),
      endLine: Math.min(
        Math.max(hit.end_line + 20, hit.end_line),
        Math.max(1, hit.end_line + 20),
      ),
    });
  }

  const fileText = contentQuery.data?.pages.map((page) => page.text).join("") ?? "";

  return (
    <section className="grid gap-3 rounded-lg border p-3" data-testid="git-source-detail">
      <header className="flex flex-wrap items-start justify-between gap-2">
        <div className="grid min-w-0 gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate text-base font-semibold">{source.owner}/{source.name}</h3>
            <GitIndexStatusBadge status={source.index_status} />
            <span className="text-xs capitalize text-muted-foreground">{source.provider}</span>
          </div>
          <p className="truncate font-mono text-xs text-muted-foreground">{source.canonical_url}</p>
          <dl className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
            <div className="flex gap-1">
              <dt className="text-muted-foreground">{t("groups.gitSources.targetSha")}:</dt>
              <dd className="font-mono">{shortSha(source.checkpoint.target_commit_sha, 12)}</dd>
            </div>
            {source.version.commit_sha ? (
              <div className="flex gap-1">
                <dt className="text-muted-foreground">{t("groups.gitSources.configuredPin")}:</dt>
                <dd className="font-mono">{shortSha(source.version.commit_sha, 12)}</dd>
              </div>
            ) : null}
            <div className="flex gap-1">
              <dt className="text-muted-foreground">{t("groups.gitSources.indexedSha")}:</dt>
              <dd className="font-mono">{shortSha(source.checkpoint.indexed_commit_sha, 12)}</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-muted-foreground">{t("groups.gitSources.skipped")}:</dt>
              <dd>{gitSkipTotal(source)}</dd>
            </div>
            <div className="flex gap-1">
              <dt className="text-muted-foreground">{t("groups.gitSources.retention")}:</dt>
              <dd>{source.retention_count}</dd>
            </div>
          </dl>
        </div>
        <div className="flex flex-wrap gap-1">
          {canManage ? (
            <>
              <Button type="button" size="sm" variant="outline" disabled={!source.enabled} onClick={onSync}>
                {t("groups.gitSources.sync")}
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={onRebuild}>
                {t("groups.gitSources.rebuild")}
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={onEdit}>
                {t("common.edit")}
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={onWebhook}>
                {t("groups.gitSources.webhook")}
              </Button>
              <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={onDelete}>
                {t("groups.gitSources.delete")}
              </Button>
            </>
          ) : null}
        </div>
      </header>

      {source.index_status === "failed" || source.last_error_code ? (
        <div role="alert" className="grid gap-1 rounded-md border border-destructive/40 p-2 text-xs">
          <span>{t("groups.gitSources.failedHint")}</span>
          {source.last_error_code ? (
            <span className="text-muted-foreground">
              {t("groups.gitSources.errorCode")}:{" "}
              <span className="font-mono" data-testid="git-source-error-code">{source.last_error_code}</span>
            </span>
          ) : null}
        </div>
      ) : null}

      {source.last_skip_summary && gitSkipTotal(source) > 0 ? (
        <div className="flex flex-wrap gap-2 text-xs" data-testid="git-skip-summary">
          {Object.entries(source.last_skip_summary)
            .filter(([, count]) => count > 0)
            .map(([reason, count]) => (
              <Badge key={reason}>{t(`groups.gitSources.skipReason.${reason}`)}: {count}</Badge>
            ))}
        </div>
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <section className="grid min-w-0 gap-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h4 className="text-sm font-medium">{t("groups.gitSources.files")}</h4>
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              {filesQuery.data ? (
                <span>
                  {t("groups.gitSources.filesSummary", {
                    count: filesQuery.data.file_count,
                    excluded: filesQuery.data.excluded_file_count,
                  })}
                </span>
              ) : null}
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={cursorStack.length === 0 || filesQuery.isFetching}
                onClick={() => {
                  setCursor(cursorStack.at(-1));
                  setCursorStack(cursorStack.slice(0, -1));
                }}
              >
                {t("groups.gitSources.previous")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={!filesQuery.data?.pagination.has_more || filesQuery.isFetching}
                onClick={() => {
                  const next = filesQuery.data?.pagination.next_cursor;
                  if (next) {
                    setCursorStack((stack) => [...stack, cursor]);
                    setCursor(next);
                  }
                }}
              >
                {t("groups.gitSources.next")}
              </Button>
            </div>
          </div>
          {filesQuery.error ? (
            <div role="alert" className="rounded-md border border-destructive/40 p-2 text-sm">
              {filesQuery.error instanceof Error ? filesQuery.error.message : t("groups.gitSources.filesLoadFailed")}
              <Button type="button" size="sm" variant="outline" className="ml-2" onClick={() => { void filesQuery.refetch(); }}>
                {t("common.retry")}
              </Button>
            </div>
          ) : null}
          <FilesTable
            files={filesQuery.data?.files ?? []}
            selectedPath={selectedPath}
            onSelect={selectFile}
          />
          {filesQuery.data ? (
            <p className="font-mono text-xs text-muted-foreground" data-testid="git-files-provenance">
              {t("groups.gitSources.generationProvenance", {
                generation: filesQuery.data.generation_number,
                commit: shortSha(filesQuery.data.commit_sha, 12),
                ref: filesQuery.data.ref_name,
              })}
            </p>
          ) : null}
        </section>

        <section className="grid min-w-0 gap-2">
          <h4 className="text-sm font-medium">{t("groups.gitSources.fileContent")}</h4>
          {!selectedPath ? (
            <p className="text-sm text-muted-foreground">{t("groups.gitSources.selectFile")}</p>
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2 text-xs">
                <span className="truncate font-mono">{selectedPath}</span>
                {fileDetailQuery.data ? (
                  <span className="text-muted-foreground">
                    {t("groups.gitSources.fileMeta", {
                      language: fileDetailQuery.data.file.language,
                      lines: fileDetailQuery.data.file.line_count,
                      bytes: fileDetailQuery.data.file.byte_count,
                    })}
                  </span>
                ) : null}
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={(contentWindow?.startLine ?? 1) <= 1}
                  onClick={() => {
                    setContentWindow((current) => {
                      const end = current?.startLine ?? MAX_CONTENT_WINDOW_LINES + 1;
                      return {
                        startLine: Math.max(1, end - MAX_CONTENT_WINDOW_LINES),
                        endLine: Math.max(1, end - 1),
                      };
                    });
                  }}
                >
                  {t("groups.gitSources.previousWindow")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    const totalLines = fileDetailQuery.data?.file.line_count ?? 0;
                    const start = windowEnd + 1;
                    if (start <= totalLines) {
                      setContentWindow({
                        startLine: start,
                        endLine: Math.min(totalLines, start + MAX_CONTENT_WINDOW_LINES - 1),
                      });
                    }
                  }}
                  disabled={!fileDetailQuery.data || windowEnd >= fileDetailQuery.data.file.line_count}
                >
                  {t("groups.gitSources.nextWindow")}
                </Button>
              </div>
              {fileDetailQuery.error ? (
                <div role="alert" className="rounded-md border border-destructive/40 p-2 text-sm">
                  {fileDetailQuery.error instanceof Error ? fileDetailQuery.error.message : t("groups.gitSources.fileLoadFailed")}
                </div>
              ) : null}
              {contentQuery.error ? (
                <div role="alert" className="rounded-md border border-destructive/40 p-2 text-sm">
                  {contentQuery.error instanceof Error ? contentQuery.error.message : t("groups.gitSources.fileLoadFailed")}
                  <Button type="button" size="sm" variant="outline" className="ml-2" onClick={() => { void contentQuery.refetch(); }}>
                    {t("common.retry")}
                  </Button>
                </div>
              ) : null}
              <p className="text-xs text-muted-foreground">
                {t("groups.gitSources.linesShown", { start: windowStart, end: windowEnd })}
              </p>
              {contentQuery.isLoading ? (
                <p className="text-sm text-muted-foreground">{t("common.loading")}</p>
              ) : (
                <pre
                  className="max-h-96 overflow-auto rounded-md border bg-muted/40 p-2 font-mono text-xs leading-5"
                  data-testid="git-file-content"
                >
                  {fileText || t("groups.gitSources.emptyContent")}
                </pre>
              )}
              {contentQuery.hasNextPage ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="justify-self-start"
                  disabled={contentQuery.isFetchingNextPage}
                  onClick={() => { void contentQuery.fetchNextPage(); }}
                >
                  {t("groups.gitSources.loadMoreContent")}
                </Button>
              ) : null}
              {contentQuery.data?.pages[0] ? (
                <p className="font-mono text-xs text-muted-foreground" data-testid="git-content-provenance">
                  {t("groups.gitSources.generationProvenance", {
                    generation: contentQuery.data.pages[0].generation_number,
                    commit: shortSha(contentQuery.data.pages[0].commit_sha, 12),
                    ref: contentQuery.data.pages[0].ref_name,
                  })}
                </p>
              ) : null}
            </>
          )}
        </section>
      </div>

      <section className="grid gap-2" data-testid="git-code-search">
        <h4 className="text-sm font-medium">{t("groups.gitSources.codeSearch")}</h4>
        <form
          className="grid gap-2 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end"
          onSubmit={(event) => {
            event.preventDefault();
            event.stopPropagation();
            void searchForm.handleSubmit();
          }}
        >
          <searchForm.Field
            name="query"
            validators={{ onChange: ({ value }) => required(value, t("groups.gitSources.required")) }}
          >
            {(field) => (
              <FormField label={t("groups.gitSources.searchQuery")} htmlFor="git-code-query">
                <Input
                  id="git-code-query"
                  value={field.state.value}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </searchForm.Field>
          <searchForm.Field name="pathPrefix">
            {(field) => (
              <FormField label={t("groups.gitSources.searchPathPrefix")} htmlFor="git-code-prefix">
                <Input
                  id="git-code-prefix"
                  value={field.state.value}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </searchForm.Field>
          <searchForm.Field name="language">
            {(field) => (
              <FormField label={t("groups.gitSources.searchLanguage")} htmlFor="git-code-language">
                <Input
                  id="git-code-language"
                  value={field.state.value}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
              </FormField>
            )}
          </searchForm.Field>
          <searchForm.Subscribe selector={(state) => [state.canSubmit, state.isSubmitting] as const}>
            {([canSubmit, isSubmitting]) => (
              <Button type="submit" disabled={!canSubmit || isSubmitting} aria-busy={isSubmitting}>
                {t("groups.gitSources.search")}
              </Button>
            )}
          </searchForm.Subscribe>
        </form>

        {searchQuery.error ? (
          <div role="alert" className="rounded-md border border-destructive/40 p-2 text-sm">
            {searchQuery.error instanceof Error ? searchQuery.error.message : t("groups.gitSources.searchFailed")}
            <Button type="button" size="sm" variant="outline" className="ml-2" onClick={() => { void searchQuery.refetch(); }}>
              {t("common.retry")}
            </Button>
          </div>
        ) : null}
        {searchQuery.data ? (
          <div className="grid gap-2">
            <p className="text-xs text-muted-foreground" data-testid="git-search-provenance">
              {t("groups.gitSources.searchProvenance", {
                generation: searchQuery.data.generation_number,
                commit: shortSha(searchQuery.data.commit_sha, 12),
                ref: searchQuery.data.ref_name,
                requested: searchQuery.data.requested_profile,
                effective: searchQuery.data.effective_profile,
              })}
              {searchQuery.data.truncated ? ` · ${t("groups.gitSources.searchTruncated")}` : ""}
              {searchQuery.data.vector_unavailable
                ? ` · ${t("groups.gitSources.vectorUnavailable", { reason: searchQuery.data.vector_unavailable })}`
                : ""}
            </p>
            {searchQuery.data.hits.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("groups.gitSources.noHits")}</p>
            ) : (
              <ul className="grid gap-2" data-testid="git-code-hits">
                {searchQuery.data.hits.map((hit) => (
                  <li key={hit.chunk_key}>
                    <button
                      type="button"
                      className="grid w-full gap-1 rounded-md border p-2 text-left hover:bg-muted/50"
                      onClick={() => selectHit(hit)}
                    >
                      <span className="flex flex-wrap items-center gap-2">
                        <span className="font-mono text-xs">{hit.path}</span>
                        <span className="text-xs text-muted-foreground">
                          {t("groups.gitSources.hitLines", { start: hit.start_line, end: hit.end_line })}
                        </span>
                        <Badge>{hit.matched}</Badge>
                        <span className="text-xs text-muted-foreground">{hit.score.toFixed(3)}</span>
                      </span>
                      <span className="line-clamp-3 whitespace-pre-wrap font-mono text-xs text-muted-foreground">
                        {hit.text}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        ) : null}
      </section>
    </section>
  );
}
