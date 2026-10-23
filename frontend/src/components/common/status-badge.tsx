import * as React from "react"

import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"

export type StatusTone =
  | "neutral"
  | "active"
  | "info"
  | "success"
  | "warning"
  | "failure"

const TONE_CLASSES: Record<StatusTone, string> = {
  neutral: "border-status-neutral-border bg-status-neutral-surface text-status-neutral-foreground",
  active: "border-status-active-border bg-status-active-surface text-status-active-foreground",
  info: "border-status-info-border bg-status-info-surface text-status-info-foreground",
  success: "border-status-success-border bg-status-success-surface text-status-success-foreground",
  warning: "border-status-warning-border bg-status-warning-surface text-status-warning-foreground",
  failure: "border-status-failure-border bg-status-failure-surface text-status-failure-foreground",
}

export type StatusBadgeProps = React.ComponentProps<"span"> & {
  tone?: StatusTone
}

export function StatusBadge({ tone = "neutral", className, ...props }: StatusBadgeProps) {
  return (
    <Badge
      variant="outline"
      data-tone={tone}
      className={cn(TONE_CLASSES[tone], className)}
      {...props}
    />
  )
}
