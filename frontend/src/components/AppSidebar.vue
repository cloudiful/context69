<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Sidebar from "primevue/sidebar";

import AppMdiIcon from "./AppMdiIcon.vue";
import AppSidebarContent from "./AppSidebarContent.vue";
import { useUiPreferences } from "../composables/use-ui-preferences";

const route = useRoute();
const { t } = useI18n();
const preferences = useUiPreferences();
const mdiChevronLeft = "M15.41,16.59L10.83,12L15.41,7.41L14,6L8,12L14,18L15.41,16.59Z";
const mdiChevronRight = "M8.59,16.59L13.17,12L8.59,7.41L10,6L16,12L10,18L8.59,16.59Z";
const mdiClose = "M18.3,5.71L12,12L5.71,5.71L4.29,7.12L10.59,13.41L4.29,19.71L5.71,21.12L12,14.83L18.3,21.12L19.71,19.71L13.41,13.41L19.71,7.12L18.3,5.71Z";

const showSidebar = computed(() => route.name !== "login");

const desktopSidebarPt = {
  mask: {
    class: "hidden md:flex",
  },
};

const mobileSidebarPt = {
  mask: {
    class: "md:hidden",
  },
};

function setMobileNavVisible(visible: boolean) {
  preferences.state.mobileNavOpen = visible;
}
</script>

<template>
  <div v-if="showSidebar">
    <Sidebar
      :visible="true"
      position="left"
      :modal="false"
      :dismissable="false"
      :show-close-icon="false"
      :close-on-escape="false"
      :auto-z-index="false"
      class="hidden md:flex"
      :class="preferences.state.sidebarCollapsed ? 'h-screen w-[4.75rem]' : 'h-screen w-[14rem]'"
      :pt="desktopSidebarPt"
    >
      <template #header>
        <div
          class="flex w-full items-center gap-2"
          :class="preferences.state.sidebarCollapsed ? 'justify-center' : 'justify-between'"
        >
          <div v-if="!preferences.state.sidebarCollapsed" class="flex items-center gap-2">
            <span class="flex h-9 w-9 items-center justify-center rounded-xl border border-app-border bg-app-surface-soft/60 text-sm font-bold">C</span>
            <span class="text-[0.98rem] font-semibold">Context69</span>
          </div>
          <Button
            data-testid="sidebar-collapse-toggle"
            class="inline-flex h-9 w-9 shrink-0 items-center justify-center px-0"
            type="button"
            severity="secondary"
            variant="outlined"
            :aria-label="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
            :title="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
            @click="preferences.toggleSidebar"
          >
            <AppMdiIcon
              :path="preferences.state.sidebarCollapsed ? mdiChevronRight : mdiChevronLeft"
              :title="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
              class="app-sidebar-link-icon h-4 w-4"
            />
          </Button>
        </div>
      </template>

      <AppSidebarContent :collapsed="preferences.state.sidebarCollapsed" />
    </Sidebar>

    <Sidebar
      :visible="preferences.state.mobileNavOpen"
      position="left"
      modal
      dismissable
      :show-close-icon="false"
      block-scroll
      class="h-screen md:hidden"
      :style="{ width: 'min(20rem, calc(100vw - 1rem))', maxWidth: 'calc(100vw - 1rem)' }"
      :pt="mobileSidebarPt"
      @update:visible="setMobileNavVisible"
    >
      <template #header>
        <div class="flex w-full items-center justify-between gap-2">
          <div class="flex items-center gap-2">
            <span class="flex h-9 w-9 items-center justify-center rounded-xl border border-app-border bg-app-surface-soft/60 text-sm font-bold">C</span>
            <span class="text-[0.98rem] font-semibold">Context69</span>
          </div>
          <Button
            class="inline-flex h-9 w-9 shrink-0 items-center justify-center px-0"
            type="button"
            severity="secondary"
            variant="outlined"
            :aria-label="t('common.close')"
            :title="t('common.close')"
            @click="setMobileNavVisible(false)"
          >
            <AppMdiIcon :path="mdiClose" :title="t('common.close')" class="app-sidebar-link-icon h-4 w-4" />
          </Button>
        </div>
      </template>

      <AppSidebarContent close-on-navigate />
    </Sidebar>
  </div>
</template>
