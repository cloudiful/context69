import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * One settings section: its page actions and content blocks. The navigation
 * already names the current page, so the section heading is screen-reader only
 * and only its action row stays visible.
 */
export function SettingsSection({
  title,
  actions,
  children,
  className,
}: {
  title: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section aria-label={title} className={cn("grid min-w-0 content-start gap-3", className)}>
      <h2 className="sr-only">{title}</h2>
      {actions ? (
        <div className="flex flex-wrap items-center justify-end gap-1.5">{actions}</div>
      ) : null}
      {children}
    </section>
  );
}

/** A titled group of related settings fields inside a section. */
export function SettingsBlock({
  id,
  title,
  actions,
  compact,
  children,
  className,
}: {
  id?: string;
  title?: string;
  actions?: ReactNode;
  compact?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section id={id} className={cn("grid gap-2 rounded-lg border bg-card p-3", className)}>
      {title || actions ? (
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="grid gap-0.5">
            {title ? <h2 className="text-sm font-semibold">{title}</h2> : null}
          </div>
          {actions ? <div className="flex flex-wrap items-center gap-1.5">{actions}</div> : null}
        </div>
      ) : null}
      <div className={cn("grid min-w-0 gap-3", compact && "gap-2")}>{children}</div>
    </section>
  );
}
