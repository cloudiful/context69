export const appFormDialogPt = {
  root: {
    class: "overflow-hidden rounded-[1.35rem] border border-app-border/90 bg-app-surface shadow-[0_20px_48px_rgba(0,0,0,0.32)]",
  },
  header: { class: "items-start px-5 pt-5 pb-4" },
  title: { class: "text-base font-semibold text-app-text" },
  content: { class: "px-5 pb-4" },
  footer: { class: "border-t border-app-border/70 px-5 pt-4 pb-5" },
} as const;

export const appConfirmDialogPt = {
  root: {
    class: "w-[min(92vw,32rem)] max-w-[calc(100vw-2rem)]",
  },
  header: { class: "px-5 pt-5 pb-3" },
  title: { class: "text-base font-semibold text-app-text" },
  content: { class: "items-start gap-3 px-5 pb-5" },
  icon: { class: "mt-0.5 shrink-0 text-xl" },
  message: { class: "min-w-0 break-words text-sm leading-6 text-app-text-muted" },
  footer: { class: "flex justify-end gap-2 border-t border-app-border/70 px-5 py-4" },
} as const;

export const libraryPreviewDialogPt = {
  header: { class: "min-w-0 border-b border-app-border/70 px-4 py-3" },
  title: { class: "min-w-0 truncate text-base font-semibold text-app-text" },
  headerActions: { class: "shrink-0" },
  content: { class: "px-4 py-3" },
} as const;
