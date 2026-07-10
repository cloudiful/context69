const SEARCH_RUNTIME_NOT_CONFIGURED_MESSAGE =
  "search runtime is not configured; save runtime settings and restart the service";

type Translate = (key: string) => string;

export function resolveSearchErrorMessage(error: unknown, t: Translate): string {
  if (!(error instanceof Error)) {
    return t("search.searchFailed");
  }

  if (error.message === SEARCH_RUNTIME_NOT_CONFIGURED_MESSAGE) {
    return t("search.runtimeNotConfigured");
  }

  return error.message || t("search.searchFailed");
}
