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
const sidebarTransitionClass = "overflow-hidden transition-[opacity,max-width,transform] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none";

const showSidebar = computed(() => route.name !== "login");

const desktopSidebarPt = {
  mask: {
    class: "hidden !pointer-events-none md:flex",
  },
};
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
      :class="[
        'hidden !h-screen !border-0 !bg-transparent !shadow-none md:flex motion-reduce:!transition-none',
        preferences.state.sidebarCollapsed
          ? '!w-[4.75rem] !transition-[width] !duration-[220ms] !ease-[cubic-bezier(0.22,1,0.36,1)]'
          : '!w-[14rem] !transition-[width] !duration-[220ms] !ease-[cubic-bezier(0.22,1,0.36,1)]',
      ]"
      :pt="desktopSidebarPt"
    >
      <template #container>
        <div class="flex h-full flex-col border-r border-(--p-content-border-color)/70 bg-(--p-content-background)/98 px-2.5 py-3 md:bg-(--p-content-hover-background)/20">
          <div class="border-b border-(--p-content-border-color)/50 pb-3">
            <div
              class="flex items-center gap-2"
              :class="preferences.state.sidebarCollapsed ? 'justify-center' : 'justify-between'"
            >
              <Transition
                :enter-active-class="sidebarTransitionClass"
                :leave-active-class="sidebarTransitionClass"
                enter-from-class="max-w-0 -translate-x-2 opacity-0"
                enter-to-class="max-w-44 translate-x-0 opacity-100"
                leave-from-class="max-w-44 translate-x-0 opacity-100"
                leave-to-class="max-w-0 -translate-x-2 opacity-0"
              >
                <div v-if="!preferences.state.sidebarCollapsed" class="flex items-center gap-2">
                  <span class="flex h-9 w-9 items-center justify-center rounded-xl border border-(--p-content-border-color) bg-(--p-content-hover-background)/60 text-sm font-bold">C</span>
                  <span class="text-[0.98rem] font-semibold">Context69</span>
                </div>
              </Transition>
              <Button
                data-testid="sidebar-collapse-toggle"
                :class="[controlButtonClass, 'inline-flex h-9 w-9 shrink-0 items-center justify-center px-0']"
                type="button"
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
          </div>

          <AppSidebarContent :collapsed="preferences.state.sidebarCollapsed" />
        </div>
      </template>
    </Sidebar>
  </div>
</template>
