import type { ToastPassThroughOptions } from "primevue/toast";

export const appToastPt = {
  root: {
    class: "w-[min(22rem,calc(100vw-2rem))]",
  },
  message: {
    class: "mb-2 overflow-hidden rounded-md border border-app-border bg-app-surface shadow-lg backdrop-blur-none",
  },
  messageContent: {
    class: "min-h-0 items-start gap-2 px-3 py-2",
  },
  messageIcon: {
    class: "mt-0.5 size-4",
  },
  messageText: {
    class: "min-w-0 gap-0",
  },
  summary: {
    class: "text-sm font-medium leading-5 text-app-text",
  },
  detail: {
    class: "mt-0.5 text-sm leading-5 text-app-text-muted",
  },
  buttonContainer: {
    class: "shrink-0",
  },
  closeButton: {
    class: "static -mr-1 -mt-1 size-7 rounded-md text-app-text-dim hover:bg-app-surface-soft hover:text-app-text focus-visible:outline-1 focus-visible:outline-offset-1 focus-visible:outline-app-border-strong",
  },
  closeIcon: {
    class: "size-3.5",
  },
} satisfies ToastPassThroughOptions;
