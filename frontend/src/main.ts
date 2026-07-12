import { createApp } from "vue";
import ConfirmationService from "primevue/confirmationservice";
import PrimeVue from "primevue/config";
import ToastService from "primevue/toastservice";

import App from "./App.vue";
import { i18n } from "./i18n";
import { router } from "./router";
import { ensureSessionReady } from "./services/auth/session";
import "primeicons/primeicons.css";
import "./style.css";

async function bootstrap() {
  await ensureSessionReady();

  createApp(App)
    .use(PrimeVue, {
      theme: {
        preset: Aura,
        options: {
          darkModeSelector: '[data-theme="dark"]',
          cssLayer: {
            name: "primevue",
            order: "tailwind-base, primevue, tailwind-utilities",
          },
        },
      },
    })
    .use(ConfirmationService)
    .use(ToastService)
    .use(router)
    .use(i18n)
    .mount("#app");
}

void bootstrap();
import Aura from "@primeuix/themes/aura";
