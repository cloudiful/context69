import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function FormField({
  label,
  htmlFor,
  helper,
  error,
  children,
  className,
}: {
  label: string;
  htmlFor?: string;
  helper?: string;
  error?: string | null;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("grid gap-1.5", className)}>
      <label htmlFor={htmlFor} className="text-sm font-medium leading-none">
        {label}
      </label>
      {children}
      {helper ? <p className="text-xs text-muted-foreground">{helper}</p> : null}
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}
