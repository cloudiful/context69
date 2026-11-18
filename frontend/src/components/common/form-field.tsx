import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export type FieldLayout = "stacked" | "row";

/**
 * Shared settings row grid: a fixed label column and the value column on
 * desktop, collapsing to one stacked column on narrow screens.
 */
export const FIELD_ROW_GRID_CLASS =
  "grid min-w-0 gap-1.5 md:grid-cols-[minmax(7rem,11rem)_minmax(0,1fr)] md:items-start md:gap-x-8 md:gap-y-0";

export const FIELD_ROW_LABEL_CLASS = "text-sm font-medium leading-6 md:pt-1.5";

export function FormField({
  label,
  htmlFor,
  helper,
  error,
  children,
  className,
  layout = "stacked",
  controlClassName,
}: {
  label: string;
  htmlFor?: string;
  helper?: string;
  error?: string | null;
  children: ReactNode;
  className?: string;
  layout?: FieldLayout;
  controlClassName?: string;
}) {
  const helperText = helper ? <p className="text-xs text-muted-foreground">{helper}</p> : null;
  const errorText = error ? (
    <p role="alert" className="text-xs text-destructive">
      {error}
    </p>
  ) : null;

  if (layout === "row") {
    return (
      <div className={cn(FIELD_ROW_GRID_CLASS, className)}>
        <label htmlFor={htmlFor} className={FIELD_ROW_LABEL_CLASS}>
          {label}
        </label>
        <div className="grid min-w-0 gap-1">
          <div className={cn("w-full max-w-md", controlClassName)}>{children}</div>
          {helperText}
          {errorText}
        </div>
      </div>
    );
  }

  return (
    <div className={cn("grid gap-1.5", className)}>
      <label htmlFor={htmlFor} className="text-sm font-medium leading-none">
        {label}
      </label>
      {children}
      {helperText}
      {errorText}
    </div>
  );
}
