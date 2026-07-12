<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import ConfirmDialog from "primevue/confirmdialog";
import Button from "primevue/button";
import SidebarLayout from "primevue/sidebarlayout";
import SidebarMain from "primevue/sidebarmain";
import Toast from "primevue/toast";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppStateMessage from "./components/AppStateMessage.vue";
import AppRouteBreadcrumbs from "./components/AppRouteBreadcrumbs.vue";
import AppSidebar from "./components/AppSidebar.vue";
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
  <ConfirmDialog :draggable="false" />
  <Toast position="bottom-right" />

  <main v-if="isLoginRoute" class="flex min-h-screen min-w-0 flex-col bg-surface-0 text-color dark:bg-surface-950">
    <RouterView />
  </main>

  <SidebarLayout v-else>
    <AppSidebar />
    <SidebarMain
      as="main"
      :class="[
        'min-w-0 px-2 py-2 pb-20 text-color md:px-3 md:pb-3',
        fillsRouteContent ? 'md:h-screen md:overflow-hidden' : 'min-h-screen',
      ]"
    >
      <div
        v-if="authSessionState.restoreError && !authSessionState.user"
        class="grid flex-1 place-items-center px-4"
      >
        <div class="grid max-w-md justify-items-center gap-3 text-center">
          <AppStateMessage severity="error" :title="$t('auth.sessionUnavailable')">
            {{ $t("auth.sessionUnavailableMessage") }}
          </AppStateMessage>
          <Button size="small" @click="retrySessionRestore">
            <i class="pi pi-refresh" aria-hidden="true" />
            <span>{{ $t("common.retry") }}</span>
          </Button>
        </div>
      </div>
      <div
        v-else-if="appReady"
        class="grid min-h-0 min-w-0 flex-1 gap-2"
        :class="showGlobalRouteBar ? 'grid-rows-[auto_minmax(0,1fr)]' : 'grid-rows-[minmax(0,1fr)]'"
      >
        <AppRouteBreadcrumbs v-if="showGlobalRouteBar" />
        <div :class="fillsRouteContent ? 'h-full min-h-0 min-w-0' : 'min-w-0 self-start'">
          <RouterView />
        </div>
      </div>
    </SidebarMain>
    <AppMobileNav />
  </SidebarLayout>
</template>
