import { render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";

const monaco = vi.hoisted(() => {
  const instances: Array<{
    container: HTMLElement;
    options: Record<string, unknown>;
    value: string;
    disposed: boolean;
    layoutCount: number;
    updateOptions: Array<Record<string, unknown>>;
    changeListener?: () => void;
  }> = [];
  const editorApi = {
    create(container: HTMLElement, options: Record<string, unknown>) {
      const instance = {
        container,
        options,
        value: String(options.value ?? ""),
        disposed: false,
        layoutCount: 0,
        updateOptions: [] as Array<Record<string, unknown>>,
        changeListener: undefined as (() => void) | undefined,
      };
      instances.push(instance);
      return {
        dispose() {
          instance.disposed = true;
        },
        getModel() {
          return { id: "model" };
        },
        getValue() {
          return instance.value;
        },
        layout() {
          instance.layoutCount += 1;
        },
        onDidChangeModelContent(listener: () => void) {
          instance.changeListener = listener;
          return { dispose() {} };
        },
        setValue(next: string) {
          instance.value = next;
        },
        updateOptions(next: Record<string, unknown>) {
          instance.updateOptions.push(next);
        },
      };
    },
    setModelLanguage: vi.fn(),
    setTheme: vi.fn(),
  };
  return { instances, editorApi };
});

vi.mock("monaco-editor", () => ({ editor: monaco.editorApi }));
vi.mock("monaco-editor/editor/editor.worker?worker", () => ({
  default: class WorkerStub {},
}));
vi.mock("@/hooks/use-preferences", () => ({
  useResolvedTheme: () => "dark",
}));

import { MonacoEditor } from "@/components/common/monaco-editor";
import { SourceConfigEditor, parseSourceDraft } from "./source-config-editor";

const VALID_CONFIG = JSON.stringify({
  source_key: "gov_documents",
  display_name: "Government documents",
  description: "",
  example_queries: ["policy"],
  connection: "primary",
  sync_strategy: "cursor",
  connector_type: "postgres_sql",
  base_query: "SELECT 1",
  batch_size: 200,
});

describe("source configuration editor", () => {
  beforeEach(() => {
    monaco.instances.length = 0;
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("projects the JSON text into form fields and writes typed edits back as JSON", async () => {
    const user = userEvent.setup();
    let latest = VALID_CONFIG;
    function Harness() {
      const [value, setValue] = useState(VALID_CONFIG);
      return (
        <SourceConfigEditor
          value={value}
          onChange={(next) => {
            latest = next;
            setValue(next);
          }}
        />
      );
    }
    render(<AppProviders><Harness /></AppProviders>);

    const sourceKey = await screen.findByLabelText("Source Key");
    expect(sourceKey).toHaveValue("gov_documents");

    await user.clear(sourceKey);
    await user.type(sourceKey, "renamed");

    expect(sourceKey).toHaveValue("renamed");
    expect(JSON.parse(latest)).toMatchObject({
      source_key: "renamed",
      connector_type: "postgres_sql",
      batch_size: 200,
      sync_strategy: "cursor",
    });
  });

  it("reports invalid JSON instead of silently replacing the text", () => {
    render(<AppProviders><SourceConfigEditor value="{ not json" onChange={() => undefined} /></AppProviders>);

    expect(screen.getByRole("alert")).toHaveTextContent("valid JSON");
    expect(screen.queryByLabelText("Source Key")).not.toBeInTheDocument();
    expect(parseSourceDraft("{ not json")).toBeNull();
  });

  it("keeps the raw text editable in JSON mode through Monaco", async () => {
    const user = userEvent.setup();
    render(<AppProviders><SourceConfigEditor value="{ not json" onChange={() => undefined} /></AppProviders>);

    await user.click(screen.getByRole("tab", { name: "JSON" }));

    await waitFor(() => expect(monaco.instances).toHaveLength(1));
    expect(monaco.instances[0].options.language).toBe("json");
    expect(monaco.instances[0].options.value).toBe("{ not json");
    expect(monaco.editorApi.setTheme).toHaveBeenCalledWith("vs-dark");
  });
});

describe("monaco editor wrapper", () => {
  beforeEach(() => {
    monaco.instances.length = 0;
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("creates one editor, forwards model edits and disposes on unmount", async () => {
    const onChange = vi.fn();
    const { unmount } = render(
      <MonacoEditor value="initial" onChange={onChange} language="json" ariaLabel="Config" />,
    );

    await waitFor(() => expect(monaco.instances).toHaveLength(1));
    expect(monaco.instances[0].options.readOnly).toBe(false);
    expect(screen.getByRole("textbox", { name: "Config" })).toHaveAttribute("aria-multiline", "true");

    monaco.instances[0].value = "typed";
    monaco.instances[0].changeListener?.();
    expect(onChange).toHaveBeenCalledWith("typed");

    unmount();
    expect(monaco.instances[0].disposed).toBe(true);
  });

  it("applies external value, read-only and language updates without recreating the editor", async () => {
    const { rerender } = render(
      <MonacoEditor value="one" onChange={() => undefined} readOnly={false} />,
    );
    await waitFor(() => expect(monaco.instances).toHaveLength(1));

    rerender(<MonacoEditor value="two" onChange={() => undefined} readOnly />);

    await waitFor(() => expect(monaco.instances[0].value).toBe("two"));
    await waitFor(() => expect(monaco.instances[0].updateOptions).toContainEqual({ readOnly: true }));
    expect(monaco.instances).toHaveLength(1);
    expect(monaco.editorApi.setModelLanguage).toHaveBeenCalledWith({ id: "model" }, "plaintext");
  });

  it("creates no zombie editor when unmounted before the module load settles", async () => {
    const { unmount } = render(
      <MonacoEditor value="initial" onChange={() => undefined} />,
    );

    unmount();

    await waitFor(() => expect(monaco.instances).toHaveLength(0));
  });
});
