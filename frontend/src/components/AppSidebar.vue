<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Sidebar from "primevue/sidebar";
import SidebarAside from "primevue/sidebaraside";
import SidebarHeader from "primevue/sidebarheader";
import SidebarPanel from "primevue/sidebarpanel";
import SidebarRail from "primevue/sidebarrail";
import SidebarSpacer from "primevue/sidebarspacer";
import SidebarTrigger from "primevue/sidebartrigger";

import AppMdiIcon from "./AppMdiIcon.vue";
import AppSidebarContent from "./AppSidebarContent.vue";
import { useUiPreferences } from "../composables/use-ui-preferences";

const route = useRoute();
const { t } = useI18n();
const preferences = useUiPreferences();
const mdiChevronLeft = "M15.41,16.59L10.83,12L15.41,7.41L14,6L8,12L14,18L15.41,16.59Z";
const mdiChevronRight = "M8.59,16.59L13.17,12L8.59,7.41L10,6L16,12L10,18L8.59,16.59Z";

const showSidebar = computed(() => route.name !== "login");
const sidebarOpen = computed({
  get: () => !preferences.state.sidebarCollapsed,
  set: (open: boolean) => {
    preferences.state.sidebarCollapsed = !open;
  },
});
</script>

<template>
  <Sidebar
    v-if="showSidebar"
    id="app-sidebar"
    v-model:open="sidebarOpen"
    class="hidden md:block"
    collapsible="icon"
    width="14rem"
    icon-width="4.75rem"
  >
    <SidebarSpacer />
    <SidebarAside>
      <SidebarPanel>
        <SidebarHeader>
          <div class="flex items-center justify-between gap-2">
            <span v-if="sidebarOpen" class="font-semibold">Context69</span>
            <SidebarTrigger v-slot="trigger" as-child>
              <Button
                type="button"
                text
                rounded
                severity="secondary"
                :class="trigger?.class"
                v-bind="trigger?.a11yAttrs"
                :aria-label="sidebarOpen ? t('sidebar.collapse') : t('sidebar.expand')"
                :title="sidebarOpen ? t('sidebar.collapse') : t('sidebar.expand')"
                data-testid="sidebar-collapse-toggle"
                @click="trigger?.onClick"
              >
                <AppMdiIcon
                  class="h-4 w-4"
                  :path="sidebarOpen ? mdiChevronLeft : mdiChevronRight"
                  :title="sidebarOpen ? t('sidebar.collapse') : t('sidebar.expand')"
                />
              </Button>
            </SidebarTrigger>
          </div>
        </SidebarHeader>
        <AppSidebarContent :collapsed="!sidebarOpen" />
      </SidebarPanel>
    </SidebarAside>
    <SidebarRail />
  </Sidebar>
</template>
