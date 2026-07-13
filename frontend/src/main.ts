import { createApp } from "vue";
import ui from "@nuxt/ui/vue-plugin";

import App from "./App.vue";
import { i18n } from "./i18n";
import { router } from "./router";
import { ensureSessionReady } from "./services/auth/session";
import "./style.css";

async function bootstrap() {
  await ensureSessionReady();

  createApp(App)
    .use(ui)
    .use(router)
    .use(i18n)
    .mount("#app");
}

void bootstrap();
