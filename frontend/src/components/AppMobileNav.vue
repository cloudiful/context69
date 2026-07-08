<script setup lang="ts">
import Button from "primevue/button";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { isAuthenticated } from "../services/auth";

const route = useRoute();
const items = useAppNavigation();
const preferences = useUiPreferences();
const { t } = useI18n();
const mdiMenu = "M3,6H21V8H3V6M3,11H21V13H3V11M3,16H21V18H3V16Z";

function isActive(itemTo: string) {
  return route.path === itemTo || route.path.startsWith(`${itemTo}/`);
}
</script>

<template>
  <nav
    v-if="route.name !== 'login' && isAuthenticated()"
    class="fixed inset-x-0 bottom-0 z-30 grid grid-cols-4 gap-1 border-t border-app-border/70 bg-app-bg/94 px-2 py-2 pb-[calc(0.5rem+env(safe-area-inset-bottom,0))] backdrop-blur-xl md:hidden"
    aria-label="Primary"
  >
    <Button
      class="grid justify-items-center gap-1 rounded-xl px-2 py-2 text-[0.72rem] font-medium text-app-text-dim transition hover:bg-app-surface-soft/70 hover:text-app-text"
      type="button"
      text
      :aria-label="t('settings.openNavigation')"
      @click="preferences.toggleMobileNav"
    >
      <span class="flex h-8 w-8 items-center justify-center rounded-lg border border-app-border/60 bg-app-surface-soft/45">
        <AppMdiIcon :path="mdiMenu" :title="t('settings.openNavigation')" class="app-sidebar-link-icon h-4 w-4" />
      </span>
      <span>{{ t("common.menu") }}</span>
    </Button>
    <RouterLink
      v-for="item in items"
      :key="item.to"
      :to="item.to"
      class="grid justify-items-center gap-1 rounded-xl px-2 py-2 text-[0.72rem] font-medium text-app-text-dim transition hover:bg-app-surface-soft/70 hover:text-app-text"
      :class="{ 'bg-app-surface-soft/70 text-app-text': isActive(item.to) }"
      :title="item.label"
    >
      <span class="flex h-8 w-8 items-center justify-center rounded-lg border border-app-border/60 bg-app-surface-soft/45">
        <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon h-4 w-4" />
      </span>
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>
