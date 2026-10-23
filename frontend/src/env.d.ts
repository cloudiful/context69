/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_API_TARGET?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
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
