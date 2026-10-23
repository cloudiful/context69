import { useEffect, useRef, useState } from "react";

import { useResolvedTheme } from "@/hooks/use-preferences";
import { cn } from "@/lib/utils";

/**
 * The subset of the Monaco editor API this wrapper drives. The module is
 * declared by the app's Vite typings without precise types, so the wrapper
 * narrows it once here instead of letting `any` leak into the app.
 */
interface MonacoDisposable {
  dispose(): void;
}

interface MonacoEditorApi {
  dispose(): void;
  getModel(): object | null;
  getValue(): string;
  layout(): void;
  onDidChangeModelContent(listener: () => void): MonacoDisposable;
  setValue(value: string): void;
  updateOptions(options: Record<string, unknown>): void;
}

interface MonacoApi {
  editor: {
    create(container: HTMLElement, options: Record<string, unknown>): MonacoEditorApi;
    setModelLanguage(model: object, language: string): void;
    setTheme(theme: string): void;
  };
}

interface MonacoWorkerModule {
  default: new () => Worker;
}

const monacoTheme = (theme: string) => (theme === "dark" ? "vs-dark" : "vs");

let workerEnvironmentInstalled = false;

/**
 * The editor bundle, its language workers and the editor worker stay out of the
 * initial application chunk: the modules load only once a wrapper mounts.
 */
async function loadMonaco(): Promise<MonacoApi> {
  const [module, workerModule, jsonWorkerModule] = await Promise.all([
    import("monaco-editor"),
    import("monaco-editor/editor/editor.worker?worker"),
    import("monaco-editor/language/json/json.worker?worker"),
  ]);
  if (!workerEnvironmentInstalled) {
    const scope = typeof self === "undefined" ? globalThis : self;
    const EditorWorker = (workerModule as MonacoWorkerModule).default;
    const JsonWorker = (jsonWorkerModule as MonacoWorkerModule).default;
    scope.MonacoEnvironment = {
      getWorker: (_workerId, label) => label === "json" ? new JsonWorker() : new EditorWorker(),
    };
    workerEnvironmentInstalled = true;
  }
  return module as unknown as MonacoApi;
}

export interface MonacoEditorProps {
  value: string;
  onChange: (value: string) => void;
  /** Stable identifier used by labels to target the editor container. */
  id?: string;
  language?: string;
  readOnly?: boolean;
  ariaLabel?: string;
  className?: string;
}

/**
 * Direct wrapper around the pinned Monaco build. It owns the editor instance,
 * the model change subscription, theming and a ResizeObserver, and disposes all
 * of them on unmount so no listener, worker layout loop or DOM node survives the
 * component. The editor module is loaded on mount, and an unmount before that
 * load settles never creates an editor.
 */
export function MonacoEditor({
  value,
  onChange,
  id,
  language = "plaintext",
  readOnly = false,
  ariaLabel,
  className,
}: MonacoEditorProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<MonacoEditorApi | null>(null);
  const applyingExternalValue = useRef(false);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  const valueRef = useRef(value);
  valueRef.current = value;
  const languageRef = useRef(language);
  languageRef.current = language;
  const readOnlyRef = useRef(readOnly);
  readOnlyRef.current = readOnly;
  const ariaLabelRef = useRef(ariaLabel);
  ariaLabelRef.current = ariaLabel;

  const theme = useResolvedTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;

  const [monaco, setMonaco] = useState<MonacoApi | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    let cancelled = false;
    let observer: ResizeObserver | null = null;
    let changeSubscription: MonacoDisposable | null = null;

    void loadMonaco().then((api) => {
      if (cancelled || !containerRef.current) {
        return;
      }

      api.editor.setTheme(monacoTheme(themeRef.current));
      const editor = api.editor.create(containerRef.current, {
        value: valueRef.current,
        language: languageRef.current,
        readOnly: readOnlyRef.current,
        automaticLayout: false,
        minimap: { enabled: false },
        scrollBeyondLastLine: false,
        wordWrap: "on",
        fontSize: 13,
        lineHeight: 22,
        padding: { top: 14, bottom: 14 },
        tabSize: 2,
        insertSpaces: true,
        suggest: { showWords: false },
        quickSuggestions: false,
        ariaLabel: ariaLabelRef.current,
      });
      if (cancelled) {
        editor.dispose();
        return;
      }

      editorRef.current = editor;
      setMonaco(api);
      changeSubscription = editor.onDidChangeModelContent(() => {
        if (applyingExternalValue.current) {
          return;
        }
        onChangeRef.current(editor.getValue());
      });
      observer = typeof ResizeObserver === "function"
        ? new ResizeObserver(() => editor.layout())
        : null;
      observer?.observe(containerRef.current);
    });

    return () => {
      cancelled = true;
      observer?.disconnect();
      changeSubscription?.dispose();
      editorRef.current?.dispose();
      editorRef.current = null;
    };
  }, []);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || editor.getValue() === value) {
      return;
    }
    applyingExternalValue.current = true;
    editor.setValue(value);
    applyingExternalValue.current = false;
  }, [value]);

  useEffect(() => {
    const model = editorRef.current?.getModel();
    if (!monaco || !model) {
      return;
    }
    monaco.editor.setModelLanguage(model, language);
  }, [language, monaco]);

  useEffect(() => {
    monaco?.editor.setTheme(monacoTheme(theme));
  }, [monaco, theme]);

  useEffect(() => {
    editorRef.current?.updateOptions({ readOnly });
  }, [readOnly]);

  return (
    <div
      id={id}
      ref={containerRef}
      role="textbox"
      aria-label={ariaLabel}
      aria-multiline="true"
      aria-readonly={readOnly || undefined}
      data-testid="monaco-editor"
      className={cn(
        "h-[min(60vh,34rem)] min-h-0 overflow-hidden rounded-lg border bg-muted/30 max-md:h-[22rem]",
        className,
      )}
    />
  );
}
