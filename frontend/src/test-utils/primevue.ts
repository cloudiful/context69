import ConfirmationService from "primevue/confirmationservice";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";
import type { App } from "vue";

if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  })) as typeof window.matchMedia;
}

export const testPrimeVuePlugin = {
  install(app: App) {
    app.use(PrimeVue, {
      theme: {
        preset: Aura,
        options: {
          darkModeSelector: '[data-theme="dark"]',
        },
      },
    });
    app.use(ConfirmationService);
    app.use(ToastService);
  },
};
import Aura from "@primeuix/themes/aura";
