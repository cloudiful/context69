import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { LibraryFileDetailResponse, SearchHit } from "@/services/api";

import type { MoveDialogState, SourceFolderState } from "./use-library-workspace";
import { SourceConfigEditor } from "./source-config-editor";

export function CreateFolderDialog({
  open,
  busy,
  parentName,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  parentName: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");

  useEffect(() => {
    if (open) {
      setName("");
    }
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("library.newFolder")}</DialogTitle>
          <DialogDescription>{parentName}</DialogDescription>
        </DialogHeader>
        <FormField label={t("library.folderName")} htmlFor="new-folder-name">
          <Input
            id="new-folder-name"
            value={name}
            autoFocus
            data-testid="new-folder-name"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && name.trim()) {
                onSubmit(name);
              }
            }}
          />
        </FormField>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !name.trim()}
            aria-busy={busy}
            onClick={() => onSubmit(name)}
          >
            {t("common.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function CreateTextFileDialog({
  open,
  busy,
  parentName,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  busy: boolean;
  parentName: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: { title: string; content: string }) => void;
}) {
  const { t } = useTranslation();
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");

  useEffect(() => {
    if (open) {
      setTitle("");
      setContent("");
    }
  }, [open]);

  const valid = title.trim().length > 0 && content.trim().length > 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("library.newTextFile")}</DialogTitle>
          <DialogDescription>{parentName}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <FormField label={t("library.textTitle")} htmlFor="new-text-title">
            <Input
              id="new-text-title"
              value={title}
              autoFocus
              data-testid="new-text-title"
              onChange={(event) => setTitle(event.target.value)}
            />
          </FormField>
          <FormField label={t("library.textContent")} htmlFor="new-text-content">
            <Textarea
              id="new-text-content"
              rows={10}
              value={content}
              data-testid="new-text-content"
              onChange={(event) => setContent(event.target.value)}
            />
          </FormField>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy || !valid}
            aria-busy={busy}
            onClick={() => onSubmit({ title, content })}
          >
            {t("common.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function MoveEntryDialog({
  state,
  busy,
  onOpenChange,
  onSubmit,
}: {
  state: MoveDialogState | null;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (targetFolderId: string | null) => void;
}) {
  const { t } = useTranslation();
  const [target, setTarget] = useState<string>("__root__");

  useEffect(() => {
    if (state) {
      setTarget(state.entry.parentFolderId ?? "__root__");
    }
  }, [state]);

  return (
    <Dialog open={!!state} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("common.move")}</DialogTitle>
          <DialogDescription>{state?.entry.name}</DialogDescription>
        </DialogHeader>
        <FormField label={t("library.targetFolder")} htmlFor="move-target-folder">
          <Select value={target} onValueChange={setTarget}>
            <SelectTrigger id="move-target-folder" className="w-full" data-testid="move-target-folder">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__root__">{t("library.rootFolder")}</SelectItem>
              {state?.options.map((option) => (
                <SelectItem key={option.value ?? "__root__"} value={option.value ?? "__root__"}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={busy}
            aria-busy={busy}
            onClick={() => onSubmit(target === "__root__" ? null : target)}
          >
            {t("common.move")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SourceFolderDialog({
  state,
  onOpenChange,
  onChangeValue,
  onChangeFolderName,
  onSubmit,
}: {
  state: SourceFolderState;
  onOpenChange: (open: boolean) => void;
  onChangeValue: (value: string) => void;
  onChangeFolderName: (value: string) => void;
  onSubmit: () => void;
}) {
  const { t } = useTranslation();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!state.open) {
      setError(null);
    }
  }, [state.open]);

  function submit() {
    try {
      const parsed: unknown = JSON.parse(state.value);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        setError(t("library.sourceConfigInvalid"));
        return;
      }
      const record = parsed as Record<string, unknown>;
      if (typeof record.source_key !== "string" || !record.source_key.trim()) {
        setError(t("library.sourceConfigMissingKey"));
        return;
      }
      setError(null);
      onSubmit();
    } catch {
      setError(t("library.sourceConfigInvalid"));
    }
  }

  return (
    <Dialog open={state.open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,60rem)] sm:max-w-[min(96vw,60rem)]">
        <DialogHeader>
          <DialogTitle>{state.title}</DialogTitle>
          <DialogDescription>{t("library.sourceConfigDescription")}</DialogDescription>
        </DialogHeader>
        <div className="grid min-w-0 gap-2">
          <SourceConfigEditor
            value={state.value}
            onChange={(next) => {
              setError(null);
              onChangeValue(next);
            }}
            disabled={state.busy}
            showFolderName={!state.folderId}
            folderName={state.folderName}
            onFolderNameChange={onChangeFolderName}
          />
          {error ? (
            <p role="alert" className="text-sm text-destructive">{error}</p>
          ) : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            disabled={state.busy || (!state.folderId && !state.folderName.trim())}
            aria-busy={state.busy}
            onClick={submit}
          >
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function PreviewDialog({
  open,
  title,
  detail,
  loading,
  error,
  canRetry,
  retrying,
  canRelease,
  releasing,
  onOpenChange,
  onRetry,
  onRelease,
}: {
  open: boolean;
  title: string;
  detail: LibraryFileDetailResponse | null;
  loading: boolean;
  error: unknown;
  canRetry: boolean;
  retrying: boolean;
  canRelease: boolean;
  releasing: boolean;
  onOpenChange: (open: boolean) => void;
  onRetry: () => void;
  onRelease: () => void;
}) {
  const { t } = useTranslation();
  const message = error instanceof Error ? error.message : error ? t("library.detailLoadFailed") : null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,64rem)] sm:max-w-[min(96vw,64rem)]">
        <DialogHeader>
          <DialogTitle className="truncate">{title}</DialogTitle>
          <DialogDescription>
            {detail ? `${detail.group_path}/${detail.folder_path}` : t("common.loading")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid max-h-[70vh] gap-3 overflow-y-auto" data-testid="library-preview">
          {loading ? <p className="text-sm text-muted-foreground">{t("common.loading")}</p> : null}
          {message ? <p role="alert" className="text-sm text-destructive">{message}</p> : null}
          {detail ? (
            <>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span>{detail.media_type}</span>
                <span>{t("library.sectionCount", { count: detail.sections.length })}</span>
                {detail.source_available === false ? (
                  <span className="text-destructive">{t("library.sourceMissing")}</span>
                ) : null}
              </div>
              {detail.sections.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("library.previewEmpty")}</p>
              ) : (
                detail.sections.map((section, index) => (
                  <section key={`${section.title}-${index}`} className="grid gap-1 rounded-md border p-3">
                    <h3 className="text-sm font-medium">{section.title || t("library.previewSection")}</h3>
                    <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words font-sans text-sm text-muted-foreground">
                      {section.preview_text}
                    </pre>
                  </section>
                ))
              )}
            </>
          ) : null}
        </div>
        <DialogFooter>
          {canRetry ? (
            <Button type="button" variant="outline" disabled={retrying} aria-busy={retrying} onClick={onRetry}>
              {t("common.retry")}
            </Button>
          ) : null}
          {canRelease ? (
            <Button type="button" variant="outline" disabled={releasing} aria-busy={releasing} onClick={onRelease}>
              {t("library.releaseSource")}
            </Button>
          ) : null}
          <Button type="button" onClick={() => onOpenChange(false)}>{t("common.close")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ScopedSearchDialog({
  open,
  query,
  folderName,
  loading,
  results,
  onOpenChange,
  onOpenResult,
}: {
  open: boolean;
  query: string;
  folderName: string;
  loading: boolean;
  results: SearchHit[];
  onOpenChange: (open: boolean) => void;
  onOpenResult: (hit: SearchHit) => void;
}) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(96vw,48rem)] sm:max-w-[min(96vw,48rem)]">
        <DialogHeader>
          <DialogTitle>{t("search.scoped.title", { folder: folderName })}</DialogTitle>
          <DialogDescription>{t("search.scoped.description", { query })}</DialogDescription>
        </DialogHeader>
        <div className="grid max-h-[60vh] gap-2 overflow-y-auto" data-testid="scoped-search-results">
          {loading ? <p className="text-sm text-muted-foreground">{t("common.loading")}</p> : null}
          {!loading && results.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("search.scoped.empty")}</p>
          ) : null}
          {results.map((hit) => (
            <button
              key={hit.chunk_id}
              type="button"
              className="grid gap-1 rounded-md border p-3 text-left hover:bg-accent"
              onClick={() => onOpenResult(hit)}
            >
              <span className="text-sm font-medium">{hit.title}</span>
              <span className="text-xs text-muted-foreground">{hit.library_path ?? hit.group_path}</span>
              <span className="line-clamp-3 text-sm">{hit.chunk_text}</span>
            </button>
          ))}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
