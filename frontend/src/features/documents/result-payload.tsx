import { useState } from "react";
import { Copy } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { formatJson } from "@/lib/format";

const TOP_LEVEL_FIELD_LIMIT = 10;
const LONG_VALUE_LIMIT = 200;
const LONG_VALUE_LINE_LIMIT = 3;

type CopyState = "idle" | "success" | "failed" | "unavailable";

function isScalar(value: unknown): value is string | number | boolean | null {
  return value === null || typeof value === "string" || typeof value === "number" || typeof value === "boolean";
}

function scalarText(value: string | number | boolean | null): string {
  return value === null ? "null" : String(value);
}

function isLongValue(value: string): boolean {
  return value.length > LONG_VALUE_LIMIT || value.split("\n").length > LONG_VALUE_LINE_LIMIT;
}

function StringValue({ value }: { value: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const long = isLongValue(value);

  if (!long) {
    return <span className="break-words whitespace-pre-wrap">{value}</span>;
  }

  return (
    <span className="grid gap-1">
      <span
        data-testid="document-result-string"
        className={
          expanded
            ? "break-words whitespace-pre-wrap"
            : "line-clamp-3 break-words whitespace-pre-wrap"
        }
      >
        {value}
      </span>
      <Button
        type="button"
        size="xs"
        variant="ghost"
        className="w-fit"
        aria-expanded={expanded}
        data-testid="document-result-string-toggle"
        onClick={() => setExpanded((current) => !current)}
      >
        {expanded ? t("document.resultShowLess") : t("document.resultShowFullValue")}
      </Button>
    </span>
  );
}

function NestedValue({ value }: { value: unknown }) {
  const { t } = useTranslation();
  const isArray = Array.isArray(value);
  const count = isArray
    ? value.length
    : Object.keys(value as Record<string, unknown>).length;
  const summary = isArray
    ? t("document.resultArraySummary", { count })
    : t("document.resultObjectSummary", { count });

  return (
    <details className="grid gap-1" data-testid="document-result-nested">
      <summary className="w-fit cursor-pointer list-none text-sm text-muted-foreground select-none">
        {summary}
      </summary>
      <pre className="max-h-48 overflow-auto rounded-lg bg-muted/50 px-3 py-2 text-xs leading-6 break-words whitespace-pre-wrap">
        {JSON.stringify(value, null, 2)}
      </pre>
    </details>
  );
}

function FieldValue({ value }: { value: unknown }) {
  if (isScalar(value)) {
    if (typeof value === "string") {
      return <StringValue value={value} />;
    }
    if (value === null) {
      return <span className="font-mono text-xs text-muted-foreground">null</span>;
    }
    return <span className="font-mono text-sm break-all">{scalarText(value)}</span>;
  }
  return <NestedValue value={value} />;
}

function RawPayload({ result }: { result: Record<string, unknown> }) {
  const { t } = useTranslation();
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const raw = formatJson(result);

  const copy = async () => {
    if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
      setCopyState("unavailable");
      return;
    }
    try {
      await navigator.clipboard.writeText(raw);
      setCopyState("success");
    } catch {
      setCopyState("failed");
    }
  };

  return (
    <details className="grid gap-1" data-testid="document-result-raw">
      <summary className="w-fit cursor-pointer list-none text-xs font-medium text-muted-foreground select-none">
        {t("document.resultRawJson")}
      </summary>
      <div className="grid gap-2 pt-1">
        <Button
          type="button"
          size="xs"
          variant="outline"
          className="w-fit"
          data-testid="document-result-copy"
          onClick={copy}
        >
          <Copy aria-hidden="true" />
          {t("document.resultCopyJson")}
        </Button>
        <p
          role="status"
          aria-live="polite"
          className="text-xs text-muted-foreground"
          data-testid="document-result-copy-status"
        >
          {copyState === "success"
            ? t("document.resultCopySuccess")
            : copyState === "failed"
              ? t("document.resultCopyFailed")
              : copyState === "unavailable"
                ? t("document.resultCopyUnavailable")
                : ""}
        </p>
        <pre
          data-testid="document-result-payload"
          className="max-h-64 overflow-auto rounded-lg bg-muted/50 px-3 py-2 text-xs leading-6 break-words whitespace-pre-wrap"
        >
          {raw}
        </pre>
      </div>
    </details>
  );
}

export function ResultPayload({ result }: { result: Record<string, unknown> }) {
  const { t } = useTranslation();
  const [showAll, setShowAll] = useState(false);
  const entries = Object.entries(result);
  const visibleEntries = showAll ? entries : entries.slice(0, TOP_LEVEL_FIELD_LIMIT);
  const hiddenCount = entries.length - visibleEntries.length;

  return (
    <div className="grid gap-2 rounded-lg border bg-card/40 px-3 py-2" data-testid="document-result">
      {entries.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="document-result-empty-payload">
          {t("document.resultEmptyPayload")}
        </p>
      ) : (
        <dl className="grid gap-1.5" data-testid="document-result-fields">
          {visibleEntries.map(([key, value]) => (
            <div key={key} className="grid gap-0.5" data-testid="document-result-field">
              <dt className="text-xs font-medium break-all text-muted-foreground">{key}</dt>
              <dd className="min-w-0 text-sm break-words">
                <FieldValue value={value} />
              </dd>
            </div>
          ))}
        </dl>
      )}
      {hiddenCount > 0 ? (
        <Button
          type="button"
          size="xs"
          variant="ghost"
          className="w-fit"
          aria-expanded={showAll}
          data-testid="document-result-show-more"
          onClick={() => setShowAll(true)}
        >
          {t("document.resultShowMoreFields", { count: hiddenCount })}
        </Button>
      ) : showAll && entries.length > TOP_LEVEL_FIELD_LIMIT ? (
        <Button
          type="button"
          size="xs"
          variant="ghost"
          className="w-fit"
          aria-expanded={showAll}
          data-testid="document-result-show-less"
          onClick={() => setShowAll(false)}
        >
          {t("document.resultShowFewerFields")}
        </Button>
      ) : null}
      <RawPayload result={result} />
    </div>
  );
}
