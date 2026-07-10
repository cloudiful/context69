<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import ConfirmDialog from "primevue/confirmdialog";
import Toast from "primevue/toast";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppRouteBreadcrumbs from "./components/AppRouteBreadcrumbs.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { appConfirmDialogPt } from "./components/app-dialog";
import { appToastPt } from "./components/app-toast";
import { useUiPreferences } from "./composables/use-ui-preferences";
import { authSessionState } from "./services/auth/session";

const preferences = useUiPreferences();
const appReady = computed(() => authSessionState.ready);
const route = useRoute();
const isLoginRoute = computed(() => route.name === "login");
const showGlobalBreadcrumbs = computed(() => !isLoginRoute.value && route.name !== "search");

onMounted(() => {
  preferences.hydrate();
});
</script>

<template>
  <div
    :class="[
      'grid min-h-screen items-start bg-app-bg text-app-text md:grid-cols-[auto_minmax(0,1fr)] md:pl-[14rem] md:transition-[padding-left] md:duration-[220ms] md:ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none',
      isLoginRoute ? 'block md:pl-0' : preferences.state.sidebarCollapsed ? 'md:pl-[4.75rem]' : '',
    ]"
  >
    <ConfirmDialog :draggable="false" :pt="appConfirmDialogPt" />
    <Toast position="bottom-right" :pt="appToastPt" />
    <AppSidebar />

    <main :class="isLoginRoute ? 'flex min-h-screen min-w-0 flex-col px-0 py-0 pb-0' : 'flex min-h-screen min-w-0 flex-col px-2 py-2 pb-20 md:px-3 md:pb-3'">
      <div v-if="appReady" class="grid min-h-0 flex-1 content-start gap-2">
        <AppRouteBreadcrumbs v-if="showGlobalBreadcrumbs" />
        <RouterView />
      </div>
    </main>
    <AppMobileNav />
  </div>
</template>
