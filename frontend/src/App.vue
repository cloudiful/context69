<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import ConfirmDialog from "primevue/confirmdialog";
import Toast from "primevue/toast";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { useUiPreferences } from "./composables/use-ui-preferences";
import { authSessionState } from "./services/auth";

const preferences = useUiPreferences();
const appReady = computed(() => authSessionState.ready);
const route = useRoute();
const isLoginRoute = computed(() => route.name === "login");

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
    <ConfirmDialog />
    <Toast position="bottom-right" />
    <AppSidebar />

    <main class="app-main-shell" :class="{ 'is-login-route': isLoginRoute }">
      <div v-if="appReady" class="flex-1">
        <RouterView />
      </div>
    </main>
    <AppMobileNav />
  </div>
</template>
