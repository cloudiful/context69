<script setup lang="ts">
import { useRoute } from "vue-router";

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
    class="fixed inset-x-0 bottom-0 z-30 grid grid-cols-4 gap-1 border-t border-surface bg-surface-0 dark:bg-surface-950 px-2 pt-2 pb-[calc(0.5rem+env(safe-area-inset-bottom,0))] backdrop-blur-xl md:hidden"
    aria-label="Primary"
  >
    <RouterLink
      v-for="item in items"
      :key="item.to"
      :to="item.to"
      :class="[
        'grid justify-items-center gap-1 rounded-xl px-2 py-2 text-[0.72rem] font-medium transition',
        isActive(item.activePrefix ?? item.to) ? 'bg-emphasis text-color' : 'text-muted-color hover:bg-emphasis hover:text-color',
      ]"
      :title="item.label"
    >
      <UIcon :name="item.icon" class="h-5 w-5" />
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>
