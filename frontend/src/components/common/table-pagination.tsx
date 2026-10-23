import { ChevronLeft, ChevronRight } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { Pagination } from "@/services/api";

const DEFAULT_PAGE_SIZE_OPTIONS = [25, 50, 100];

export function TablePagination({
  pagination,
  onPage,
  onPageSize,
  pageSizeOptions = DEFAULT_PAGE_SIZE_OPTIONS,
}: {
  pagination: Pagination;
  onPage: (page: number) => void;
  onPageSize: (pageSize: number) => void;
  pageSizeOptions?: readonly number[];
}) {
  const { t } = useTranslation();
  const pageSizeId = useId();
  const totalPages = Math.max(1, pagination.total_pages);
  const canPrev = pagination.page > 1;
  const canNext = pagination.page < totalPages;

  return (
    <div className="flex flex-wrap items-center justify-end gap-2 pt-2 text-sm">
      <label htmlFor={pageSizeId} className="text-xs text-muted-foreground">
        {t("common.itemsPerPage")}
      </label>
      <Select
        value={String(pagination.page_size)}
        onValueChange={(value) => onPageSize(Number(value))}
      >
        <SelectTrigger id={pageSizeId} size="sm" className="w-20" aria-label={t("common.itemsPerPage")}>
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
      <span className="text-xs text-muted-foreground">
        {t("common.pageOf", { page: pagination.page, totalPages })}
      </span>
      <Button
        type="button"
        variant="outline"
        size="icon-sm"
        aria-label={t("common.previous")}
        disabled={!canPrev}
        onClick={() => onPage(pagination.page - 1)}
      >
        <ChevronLeft aria-hidden="true" />
      </Button>
      <Button
        type="button"
        variant="outline"
        size="icon-sm"
        aria-label={t("common.next")}
        disabled={!canNext}
        onClick={() => onPage(pagination.page + 1)}
      >
        <ChevronRight aria-hidden="true" />
      </Button>
    </div>
  );
}
