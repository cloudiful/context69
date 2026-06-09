/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_API_TARGET?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

declare module "*.vue" {
  import type { DefineComponent } from "vue";

  const component: DefineComponent<Record<string, never>, Record<string, never>, unknown>;
  export default component;
}

declare module "monaco-editor" {
  export const editor: any;
}

declare module "monaco-editor/esm/vs/editor/editor.worker?worker" {
  const MonacoEditorWorker: {
    new (): Worker;
  };

  export default MonacoEditorWorker;
}

interface Window {
  MonacoEnvironment?: {
    getWorker: () => Worker;
  };
}

interface GlobalThis {
  MonacoEnvironment?: {
    getWorker: () => Worker;
  };
}
