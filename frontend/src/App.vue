<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import ConfirmDialog from "primevue/confirmdialog";
import Button from "primevue/button";
import Toast from "primevue/toast";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppStateMessage from "./components/AppStateMessage.vue";
import AppRouteBreadcrumbs from "./components/AppRouteBreadcrumbs.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { appConfirmDialogPt } from "./components/app-dialog";
import { appToastPt } from "./components/app-toast";
import { useUiPreferences } from "./composables/use-ui-preferences";
import { authSessionState, restoreSession } from "./services/auth/session";

const preferences = useUiPreferences();
const appReady = computed(() => authSessionState.ready);
const route = useRoute();
const isLoginRoute = computed(() => route.name === "login");
const showGlobalRouteBar = computed(() => {
  const routeName = String(route.name ?? "");
  return routeName === "group-overview"
    || routeName === "group-members"
    || routeName === "group-settings"
    || routeName.startsWith("settings-");
});
const fillsRouteContent = computed(() => route.meta.contentLayout === "fill");

async function retrySessionRestore() {
  if (await restoreSession()) window.location.reload();
}

onMounted(() => {
  preferences.hydrate();
});
</script>

<template>
  <div
    :class="isLoginRoute
      ? 'block min-h-screen bg-app-bg text-app-text'
      : [
        'grid min-h-screen items-start bg-app-bg text-app-text md:grid-cols-[auto_minmax(0,1fr)] md:pl-[14rem] md:transition-[padding-left] md:duration-[220ms] md:ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none',
        preferences.state.sidebarCollapsed ? 'md:pl-[4.75rem]' : '',
      ]"
  >
    <ConfirmDialog :draggable="false" :pt="appConfirmDialogPt" />
    <Toast position="bottom-right" :pt="appToastPt" />
    <AppSidebar />

    <main :class="isLoginRoute ? 'flex min-h-screen min-w-0 flex-col px-0 py-0 pb-0' : 'flex min-h-screen min-w-0 flex-col px-2 py-2 pb-20 md:px-3 md:pb-3'">
      <div
        v-if="authSessionState.restoreError && !authSessionState.user"
        class="grid flex-1 place-items-center px-4"
      >
        <div class="grid max-w-md justify-items-center gap-3 text-center">
          <AppStateMessage severity="error" :title="$t('auth.sessionUnavailable')">
            {{ $t("auth.sessionUnavailableMessage") }}
          </AppStateMessage>
          <Button size="small" icon="pi pi-refresh" :label="$t('common.retry')" @click="retrySessionRestore" />
        </div>
      </div>
      <div
        v-else-if="appReady"
        class="grid min-h-0 flex-1 gap-2"
        :class="showGlobalRouteBar ? 'grid-rows-[auto_minmax(0,1fr)]' : 'grid-rows-[minmax(0,1fr)]'"
      >
        <AppRouteBreadcrumbs v-if="showGlobalRouteBar" />
        <div :class="fillsRouteContent ? 'h-full min-h-0' : 'min-w-0 self-start'">
          <RouterView />
        </div>
      </div>
    </main>
    <AppMobileNav />
  </div>
</template>
