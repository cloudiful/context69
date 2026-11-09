import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/** One settings section: its heading and content blocks. */
export function SettingsSection({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section aria-label={title} className={cn("grid min-w-0 content-start gap-3", className)}>
      <div className="grid gap-0.5">
        <h2 className="text-base font-semibold">{title}</h2>
      </div>
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
