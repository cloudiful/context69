<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Sidebar from "primevue/sidebar";

import AppMdiIcon from "./AppMdiIcon.vue";
import AppSidebarContent from "./AppSidebarContent.vue";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { controlButtonClass } from "../ui/button-classes";

const route = useRoute();
const { t } = useI18n();
const preferences = useUiPreferences();
const mdiChevronLeft = "M15.41,16.59L10.83,12L15.41,7.41L14,6L8,12L14,18L15.41,16.59Z";
const mdiChevronRight = "M8.59,16.59L13.17,12L8.59,7.41L10,6L16,12L10,18L8.59,16.59Z";
const mdiClose = "M18.3,5.71L12,12L5.71,5.71L4.29,7.12L10.59,13.41L4.29,19.71L5.71,21.12L12,14.83L18.3,21.12L19.71,19.71L13.41,13.41L19.71,7.12L18.3,5.71Z";

const showSidebar = computed(() => route.name !== "login");

const desktopSidebarPt = {
  mask: {
    class: "app-sidebar-shell-mask app-sidebar-shell-mask-desktop hidden md:flex",
  },
};

const mobileSidebarPt = {
  mask: {
    class: "app-sidebar-shell-mask app-sidebar-shell-mask-mobile md:hidden",
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
      class="app-sidebar-shell app-sidebar-shell-desktop hidden md:flex"
      :class="{ 'is-collapsed': preferences.state.sidebarCollapsed }"
      :pt="desktopSidebarPt"
    >
      <template #container>
        <div class="app-sidebar-panel">
          <div class="app-sidebar-head">
            <div
              class="flex items-center gap-2"
              :class="preferences.state.sidebarCollapsed ? 'justify-center' : 'justify-between'"
            >
              <Transition name="sidebar-brand">
                <div v-if="!preferences.state.sidebarCollapsed" class="app-sidebar-brand">
                  <span class="app-sidebar-brand-mark">C</span>
                  <span class="app-sidebar-brand-text">Context69</span>
                </div>
              </Transition>
              <Button
                data-testid="sidebar-collapse-toggle"
                :class="[controlButtonClass, 'app-sidebar-icon-button']"
                type="button"
                :aria-label="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
                :title="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
                @click="preferences.toggleSidebar"
              >
                <AppMdiIcon
                  :path="preferences.state.sidebarCollapsed ? mdiChevronRight : mdiChevronLeft"
                  :title="preferences.state.sidebarCollapsed ? t('sidebar.expand') : t('sidebar.collapse')"
                  class="app-sidebar-link-icon"
                />
              </Button>
            </div>
          </div>

          <AppSidebarContent :collapsed="preferences.state.sidebarCollapsed" />
        </div>
      </template>
    </Sidebar>

    <Sidebar
      :visible="preferences.state.mobileNavOpen"
      position="left"
      modal
      dismissable
      :show-close-icon="false"
      block-scroll
      class="app-sidebar-shell app-sidebar-shell-mobile md:hidden"
      :pt="mobileSidebarPt"
      @update:visible="setMobileNavVisible"
    >
      <template #container="{ closeCallback }">
        <div class="app-sidebar-panel app-sidebar-panel-mobile">
          <div class="app-sidebar-head">
            <div class="flex items-center justify-between gap-2">
              <div class="app-sidebar-brand">
                <span class="app-sidebar-brand-mark">C</span>
                <span class="app-sidebar-brand-text">Context69</span>
              </div>
              <Button
                :class="[controlButtonClass, 'app-sidebar-icon-button']"
                type="button"
                :aria-label="t('common.close')"
                :title="t('common.close')"
                @click="closeCallback"
              >
                <AppMdiIcon :path="mdiClose" :title="t('common.close')" class="app-sidebar-link-icon" />
              </Button>
            </div>
          </div>

          <AppSidebarContent close-on-navigate />
        </div>
      </template>
    </Sidebar>
  </div>
</template>
