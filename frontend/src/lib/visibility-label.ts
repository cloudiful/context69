import type { Visibility } from "@/services/api";

type Translate = (key: string, params?: Record<string, unknown>) => string;

const VISIBILITY_VALUES: readonly Visibility[] = ["private", "public"];

/**
 * The one visibility label used by groups, the library and the document reader.
 * Known generated values resolve through the shared catalog; a value this build
 * does not know stays visible as its raw code so an unexpected server value is
 * never hidden or mistaken for a permission. The returned text is display only:
 * requests, filters and permission gates keep the typed enum.
 */
export function visibilityLabel(
  t: Translate,
  visibility: Visibility | null | undefined,
): string {
  if (!visibility) {
    return "--";
  }
  return VISIBILITY_VALUES.includes(visibility)
    ? t(`groups.visibilityOptions.${visibility}`)
    : visibility;
}
