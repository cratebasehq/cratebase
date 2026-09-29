/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_CRATEBASE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
