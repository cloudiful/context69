<script setup lang="ts">
import Button from "primevue/button";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { isAuthenticated } from "../services/auth/session";

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
    class="app-mobile-nav"
    aria-label="Primary"
  >
    <Button
      class="app-mobile-nav-link app-mobile-nav-menu"
      type="button"
      text
      :aria-label="t('settings.openNavigation')"
      @click="preferences.toggleMobileNav"
    >
      <span class="app-mobile-nav-mark">
        <AppMdiIcon :path="mdiMenu" :title="t('settings.openNavigation')" class="app-sidebar-link-icon" />
      </span>
      <span>{{ t("common.menu") }}</span>
    </Button>
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
