export const appContextMenuPt = {
  root: {
    class: "min-w-32 overflow-visible rounded-xl border border-app-border/70 bg-app-surface/96 p-1 text-app-text shadow-[0_18px_48px_rgba(0,0,0,0.34)] backdrop-blur-xl",
  },
  rootList: {
    class: "m-0 flex list-none flex-col gap-0.5 p-0 outline-none",
  },
  item: {
    class: "relative list-none",
  },
  itemContent: {
    class: "rounded-lg",
  },
  itemLink: {
    class: "flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm leading-5 text-app-text no-underline transition hover:bg-app-surface-soft/72 hover:text-app-text focus:bg-app-surface-soft/72 focus:text-app-text focus:outline-none aria-disabled:pointer-events-none aria-disabled:opacity-50",
  },
  itemIcon: {
    class: "text-xs text-app-text-dim",
  },
  itemLabel: {
    class: "truncate",
  },
  submenuIcon: {
    class: "ml-auto text-[0.7rem] text-app-text-dim",
  },
  submenu: {
    class: "absolute left-full top-0 z-20 ml-1 min-w-32 overflow-visible rounded-xl border border-app-border/70 bg-app-surface/96 p-1 text-app-text shadow-[0_18px_48px_rgba(0,0,0,0.34)] backdrop-blur-xl",
  },
  separator: {
    class: "my-1 border-t border-app-border/60",
  },
};
