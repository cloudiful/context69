<script setup lang="ts">
import { useRoute } from "vue-router";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { isAuthenticated } from "../services/auth/session";

const route = useRoute();
const items = useAppNavigation();

function isActive(itemTo: string) {
  return route.path === itemTo || route.path.startsWith(`${itemTo}/`);
}
</script>

<template>
  <nav
    v-if="route.name !== 'login' && isAuthenticated()"
    class="fixed inset-x-0 bottom-0 z-30 grid grid-cols-3 gap-1 border-t border-app-border/70 bg-app-bg/94 px-2 pt-2 pb-[calc(0.5rem+env(safe-area-inset-bottom,0))] backdrop-blur-xl md:hidden"
    aria-label="Primary"
  >
    <RouterLink
      v-for="item in items"
      :key="item.to"
      :to="item.to"
      :class="[
        'grid justify-items-center gap-1 rounded-xl px-2 py-2 text-[0.72rem] font-medium transition',
        isActive(item.activePrefix ?? item.to) ? 'bg-app-surface-soft/70 text-app-text' : 'text-app-text-dim hover:bg-app-surface-soft/70 hover:text-app-text',
      ]"
      :title="item.label"
    >
      <span class="flex h-8 w-8 items-center justify-center rounded-lg border border-app-border/60 bg-app-surface-soft/45">
        <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon h-4 w-4" />
      </span>
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>
