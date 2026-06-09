<script setup lang="ts">
import { computed, onMounted } from "vue";
import ConfirmDialog from "primevue/confirmdialog";
import Toast from "primevue/toast";

import AppContextBar from "./components/AppContextBar.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { useUiPreferences } from "./composables/use-ui-preferences";
import { authSessionState } from "./services/auth";

const preferences = useUiPreferences();
const appReady = computed(() => authSessionState.ready);

onMounted(() => {
  preferences.hydrate();
});
</script>

<template>
  <div class="app-shell">
    <ConfirmDialog />
    <Toast position="bottom-right" />
    <AppSidebar />

    <main class="app-main-shell">
      <AppContextBar />
      <div v-if="appReady" class="flex-1">
        <RouterView />
      </div>
    </main>
  </div>
</template>
