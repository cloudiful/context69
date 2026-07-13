<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import AppSidebarContent from "./AppSidebarContent.vue";
import { useUiPreferences } from "../composables/use-ui-preferences";

const { t } = useI18n();
const preferences = useUiPreferences();
const collapsed = computed({
  get: () => preferences.state.sidebarCollapsed,
  set: (value: boolean) => {
    preferences.state.sidebarCollapsed = value;
  },
});
</script>

<template>
  <UDashboardSidebar
    id="app-sidebar"
    v-model:collapsed="collapsed"
    class="hidden md:flex"
    collapsible
    resizable
    :default-size="14"
    :min-size="12"
    :max-size="20"
    :collapsed-size="4.75"
  >
    <template #header="{ collapsed: isCollapsed }">
      <div class="flex w-full items-center justify-between gap-2">
        <span v-if="!isCollapsed" class="font-semibold">Context69</span>
        <UDashboardSidebarCollapse
          :aria-label="isCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
          :title="isCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
          data-testid="sidebar-collapse-toggle"
        />
      </div>
    </template>
    <template #default="{ collapsed: isCollapsed }">
      <AppSidebarContent :collapsed="isCollapsed" />
    </template>
  </UDashboardSidebar>
</template>
