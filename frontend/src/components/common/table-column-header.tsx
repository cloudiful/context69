import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type ColumnSortDirection = "asc" | "desc" | "none";

export interface TableColumnHeaderProps {
  label: string;
  /** Sort action for this column; the backend decides which fields are sortable. */
  sort?: {
    direction: ColumnSortDirection;
    onSort: () => void;
    ariaLabel: string;
  };
  /** Optional filter action rendered as a separate icon control. */
  filter?: ReactNode;
  className?: string;
}

const SORT_ICONS = {
  asc: ArrowUp,
  desc: ArrowDown,
  none: ChevronsUpDown,
} as const;

/**
 * One column title with optional, independent sort and filter actions. A
 * sortable label is one native button whose text and indicator share the sort
 * target; an unsortable label stays plain text. The filter remains a separate
 * control, so a filtered header stays visually marked.
 */
export function TableColumnHeader({ label, sort, filter, className }: TableColumnHeaderProps) {
  const SortIcon = sort ? SORT_ICONS[sort.direction] : null;

  return (
    <div data-slot="column-header" className={cn("flex h-8 items-center gap-0.5", className)}>
      {sort && SortIcon
        ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-6 px-1 font-medium"
              data-sort={sort.direction}
              aria-label={sort.ariaLabel}
              onClick={sort.onSort}
            >
              {label}
              <SortIcon
                aria-hidden="true"
                className={cn("size-3", sort.direction === "none" ? "opacity-40" : "opacity-100")}
              />
            </Button>
          )
        : <span className="font-medium">{label}</span>}
      {filter}
    </div>
  );
}
