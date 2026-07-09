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
    class="app-mobile-nav"
    aria-label="Primary"
  >
    <RouterLink
      v-for="item in items"
      :key="item.to"
      :to="item.to"
      class="app-mobile-nav-link"
      :class="{ 'is-active': isActive(item.to) }"
      :title="item.label"
    >
      <span class="app-mobile-nav-mark">
        <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon" />
      </span>
      <span>{{ item.label }}</span>
    </RouterLink>
  </nav>
</template>
