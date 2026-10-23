import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link, useLocation } from "react-router";

import { SEGMENT_LABEL_KEYS } from "./app-nav";

/**
 * Decodes one raw URL path segment for display. A malformed percent escape is
 * shown as it appears instead of throwing, so an odd URL cannot break the
 * shell.
 */
export function decodePathSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/**
 * Splits the raw pathname so an encoded slash stays inside one segment, then
 * keeps each original encoded segment for link construction. Decoding the whole
 * pathname first would turn `stock%2Falpha` into two labels and link to a group
 * that does not exist.
 */
export function breadcrumbTrail(pathname: string): Array<{ segment: string; href: string }> {
  const segments = pathname.split("/").filter(Boolean);

  return segments.map((segment, index) => ({
    segment,
    href: `/${segments.slice(0, index + 1).join("/")}`,
  }));
}

/** Section roots that have no index route; their label stays plain text. */
const NON_NAVIGABLE_SEGMENTS = new Set(["documents"]);

export function AppBreadcrumbs() {
  const { t } = useTranslation();
  const location = useLocation();
  const trail = breadcrumbTrail(location.pathname);

  return (
    <nav
      aria-label={t("app.breadcrumbs")}
      data-testid="app-breadcrumbs"
      className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground"
    >
      <Link to="/" className="truncate hover:text-foreground">
        {t("app.title")}
      </Link>
      {trail.map(({ segment, href }, index) => {
        const labelKey = SEGMENT_LABEL_KEYS[segment];
        const isLast = index === trail.length - 1;
        const label = labelKey ? t(labelKey) : decodePathSegment(segment);

        return (
          <span key={href} className="flex min-w-0 items-center gap-1">
            <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />
            {isLast || NON_NAVIGABLE_SEGMENTS.has(segment) ? (
              <span className={isLast ? "truncate text-foreground" : "truncate"}>{label}</span>
            ) : (
              <Link to={href} className="truncate hover:text-foreground">
                {label}
              </Link>
            )}
          </span>
        );
      })}
    </nav>
  );
}
