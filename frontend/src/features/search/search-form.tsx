import { Clock, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { GroupResponse, SearchSort, SourceStatus } from "@/services/api";
import type { SearchFilters } from "@/types/ui";
import type { SearchHistoryEntry } from "@/utils/search-history";

import { GROUP_FOLDER_ALL_VALUE, groupFolderOptions } from "@/utils/search";

const ALL_SOURCES_VALUE = "__all__";

export function SearchForm({
  filters,
  sources,
  groups,
  history,
  busy,
  onChange,
  onSubmit,
  onGroupChange,
  onHistorySelect,
  onReset,
}: {
  filters: SearchFilters;
  sources: SourceStatus[];
  groups: GroupResponse[];
  history: SearchHistoryEntry[];
  busy: boolean;
  onChange: (filters: SearchFilters) => void;
  onSubmit: () => void;
  onGroupChange: (groupPath: string) => void;
  onHistorySelect: (entry: SearchHistoryEntry) => void;
  onReset: () => void;
}) {
  const { t } = useTranslation();
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);

  useEffect(() => {
    if (filters.publishedAfter || filters.publishedBefore || filters.sort === "date") {
      setAdvancedOpen(true);
    }
  }, [filters.publishedAfter, filters.publishedBefore, filters.sort]);

  const patch = (next: Partial<SearchFilters>) => {
    onChange({ ...filters, ...next });
  };

  const folderOptions = groupFolderOptions(groups);
  const sourceOptions = [
    { value: ALL_SOURCES_VALUE, label: t("search.form.allSources") },
    ...sources.map((source) => ({
      value: source.source_key,
      label: source.display_name === source.source_key
        ? source.display_name
        : `${source.display_name} (${source.source_key})`,
    })),
  ];

  return (
    <section className="min-w-0 shrink-0">
      <form
        className="grid min-w-0 gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <div className="grid min-w-0 items-center gap-2 lg:grid-cols-[minmax(0,1fr)_auto]">
          <div className="flex min-w-0 items-center gap-1.5">
            <div className="relative min-w-0 flex-1">
              <Search
                aria-hidden="true"
                className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                id="search-query"
                data-testid="search-query"
                className="pl-8"
                value={filters.query}
                placeholder={t("search.form.query")}
                aria-label={t("search.form.query")}
                autoComplete="off"
                onChange={(event) => patch({ query: event.target.value })}
              />
            </div>
            <Popover open={historyOpen} onOpenChange={setHistoryOpen}>
              <PopoverTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label={t("search.history.title")}
                  title={t("search.history.title")}
                  data-testid="search-history"
                >
                  <Clock aria-hidden="true" />
                </Button>
              </PopoverTrigger>
              <PopoverContent align="end" className="w-80 p-1">
                <p className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                  {t("search.history.title")}
                </p>
                {history.length === 0 ? (
                  <p className="px-2 py-1.5 text-sm text-muted-foreground">
                    {t("search.history.empty")}
                  </p>
                ) : (
                  <ul className="grid max-h-72 gap-0.5 overflow-y-auto">
                    {history.map((entry) => (
                      <li key={`${entry.query}-${entry.sourceKey}-${entry.groupPath ?? ""}-${entry.limit}-${entry.sort ?? "relevance"}`}>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="w-full justify-start font-normal"
                          data-testid="search-history-entry"
                          onClick={() => {
                            setHistoryOpen(false);
                            onHistorySelect(entry);
                          }}
                        >
                          <span className="truncate">{entry.query}</span>
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
              </PopoverContent>
            </Popover>
          </div>

          <div className="flex min-w-0 items-stretch justify-end gap-1.5">
            <Button
              type="button"
              variant="ghost"
              data-testid="search-toggle-advanced"
              aria-expanded={advancedOpen}
              onClick={() => setAdvancedOpen((open) => !open)}
            >
              {advancedOpen ? t("search.form.hideFilters") : t("search.form.moreFilters")}
            </Button>
            <Button
              type="submit"
              className="min-w-28"
              data-testid="search-submit"
              disabled={busy}
              aria-busy={busy}
            >
              {busy ? t("search.form.running") : t("search.form.run")}
            </Button>
          </div>
        </div>

        {filters.sort === "date" ? (
          <p
            data-testid="search-sort-mode"
            className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground"
          >
            <Clock aria-hidden="true" className="size-3.5 shrink-0" />
            <span className="truncate">{t("search.form.sortDateLabel")}</span>
          </p>
        ) : null}

        {advancedOpen ? (
          <div className="grid min-w-0 gap-3 border-t pt-3">
            <div className="grid min-w-0 gap-3 sm:grid-cols-2 lg:grid-cols-[minmax(0,1fr)_8rem]">
              <FormField label={t("search.form.source")} htmlFor="search-source">
                <Select
                  value={filters.sourceKey || ALL_SOURCES_VALUE}
                  onValueChange={(value) => patch({ sourceKey: value === ALL_SOURCES_VALUE ? "" : value })}
                >
                  <SelectTrigger id="search-source" data-testid="search-source" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {sourceOptions.map((source) => (
                      <SelectItem key={source.value} value={source.value}>
                        <span className="block max-w-[min(80vw,20rem)] truncate">{source.label}</span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FormField>

              <FormField label={t("search.form.limit")} htmlFor="search-limit">
                <Input
                  id="search-limit"
                  data-testid="search-limit"
                  type="number"
                  min={1}
                  max={50}
                  value={String(filters.limit)}
                  onChange={(event) => {
                    const parsed = Number.parseInt(event.target.value, 10);
                    if (!Number.isFinite(parsed)) {
                      return;
                    }
                    patch({ limit: Math.min(Math.max(parsed, 1), 50) });
                  }}
                />
              </FormField>
            </div>

            <FormField label={t("search.form.sort")} htmlFor="search-sort">
              <Select
                value={filters.sort ?? "relevance"}
                onValueChange={(value) => patch({ sort: value as SearchSort })}
              >
                <SelectTrigger id="search-sort" data-testid="search-sort" className="w-full sm:max-w-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="relevance">{t("search.form.sortRelevance")}</SelectItem>
                  <SelectItem value="date">{t("search.form.sortDate")}</SelectItem>
                </SelectContent>
              </Select>
            </FormField>

            <FormField label={t("search.form.groupPath")} htmlFor="search-group-path">
              <Select
                value={filters.groupPath || GROUP_FOLDER_ALL_VALUE}
                onValueChange={(value) => {
                  const groupPath = value === GROUP_FOLDER_ALL_VALUE ? "" : value;
                  patch({ groupPath });
                  onGroupChange(groupPath);
                }}
              >
                <SelectTrigger id="search-group-path" data-testid="search-group-path" className="w-full sm:max-w-md">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={GROUP_FOLDER_ALL_VALUE}>{t("search.form.allGroups")}</SelectItem>
                  {folderOptions.map((group) => (
                    <SelectItem key={group.value} value={group.value}>
                      <span
                        className="block max-w-[min(80vw,26rem)] truncate"
                        style={{ paddingLeft: `${group.depth * 12}px` }}
                      >
                        {group.label}
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </FormField>

            <div className="grid gap-3 sm:max-w-md" data-testid="search-published-range">
              <div className="grid gap-3 sm:grid-cols-2">
                <FormField label={t("search.form.publishedAfter")} htmlFor="search-published-after">
                  <Input
                    id="search-published-after"
                    data-testid="search-published-after"
                    type="date"
                    value={filters.publishedAfter}
                    onChange={(event) => patch({ publishedAfter: event.target.value })}
                  />
                </FormField>
                <FormField label={t("search.form.publishedBefore")} htmlFor="search-published-before">
                  <Input
                    id="search-published-before"
                    data-testid="search-published-before"
                    type="date"
                    value={filters.publishedBefore}
                    onChange={(event) => patch({ publishedBefore: event.target.value })}
                  />
                </FormField>
              </div>
            </div>

            <div className="flex justify-end">
              <Button
                type="button"
                variant="outline"
                data-testid="search-reset"
                onClick={() => {
                  setAdvancedOpen(false);
                  onReset();
                }}
              >
                {t("common.reset")}
              </Button>
            </div>
          </div>
        ) : null}
      </form>
    </section>
  );
}
