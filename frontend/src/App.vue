<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import ConfirmDialog from "primevue/confirmdialog";
import Toast from "primevue/toast";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppRouteBreadcrumbs from "./components/AppRouteBreadcrumbs.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { appConfirmDialogPt } from "./components/app-dialog";
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
    class="app-shell"
    :class="{
      'is-login-route': isLoginRoute,
      'is-sidebar-collapsed': preferences.state.sidebarCollapsed,
    }"
  >
    <ConfirmDialog :draggable="false" :pt="appConfirmDialogPt" />
    <Toast position="bottom-right" />
    <AppSidebar />

    <main class="app-main-shell" :class="{ 'is-login-route': isLoginRoute }">
      <div v-if="appReady" class="app-main-content">
        <AppRouteBreadcrumbs v-if="showGlobalBreadcrumbs" />
        <RouterView />
      </div>
    </main>
    <AppMobileNav />
  </div>
</template>
