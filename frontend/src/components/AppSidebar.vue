<script setup lang="ts">
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Tag from "primevue/tag";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { authSessionState, isAuthenticated, logout } from "../services/auth";
import { useUiPreferences } from "../composables/use-ui-preferences";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const preferences = useUiPreferences();
const items = useAppNavigation();
const mdiLogoutVariant = "M14.08,15.59L16.67,13H7V11H16.67L14.08,8.41L15.5,7L20.5,12L15.5,17L14.08,15.59M4,19H10V21H4C2.89,21 2,20.1 2,19V5C2,3.89 2.89,3 4,3H10V5H4V19Z";
const mdiChevronLeft = "M15.41,16.59L10.83,12L15.41,7.41L14,6L8,12L14,18L15.41,16.59Z";
const mdiChevronRight = "M8.59,16.59L13.17,12L8.59,7.41L10,6L16,12L10,18L8.59,16.59Z";

function isActive(itemTo: string) {
  return route.path === itemTo || route.path.startsWith(`${itemTo}/`);
}

async function signOut() {
  await logout();
  preferences.closeMobileNav();
  await router.replace({
    name: "login",
  });
}
</script>

<template>
  <div v-if="route.name !== 'login'">
    <button
      v-if="preferences.state.mobileNavOpen"
      class="app-sidebar-backdrop"
      type="button"
      aria-label="Close navigation"
      @click="preferences.closeMobileNav"
    />

    <aside
      class="app-sidebar"
      :class="{
        'is-collapsed': preferences.state.sidebarCollapsed,
        'is-mobile-open': preferences.state.mobileNavOpen,
      }"
    >
      <div class="app-sidebar-head">
        <div class="flex items-center justify-between gap-2">
          <div class="app-sidebar-brand">
            <span class="app-sidebar-brand-mark">C</span>
            <span v-if="!preferences.state.sidebarCollapsed" class="app-sidebar-brand-text">Context69</span>
          </div>
          <Button
            v-if="!preferences.state.mobileNavOpen"
            data-testid="sidebar-collapse-toggle"
            class="app-control-button app-sidebar-icon-button hidden md:inline-flex"
            type="button"
            text
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

      <nav class="app-sidebar-nav">
        <RouterLink
          v-for="item in isAuthenticated() ? items : []"
          :key="item.to"
          :to="item.to"
          class="app-sidebar-link"
          :class="{ 'is-active': isActive(item.to) }"
          :data-nav-key="item.to"
          :title="item.label"
          @click="preferences.closeMobileNav"
        >
          <span class="app-sidebar-link-mark">
            <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon" />
          </span>
          <span v-if="!preferences.state.sidebarCollapsed">{{ item.label }}</span>
        </RouterLink>
      </nav>

      <div class="app-sidebar-footer">
        <div class="flex items-end gap-2">
          <div
            v-if="authSessionState.user && !preferences.state.sidebarCollapsed"
            class="app-sidebar-user grow"
            data-testid="sidebar-user-card"
          >
            <div class="flex items-start justify-between gap-2">
              <span data-testid="sidebar-user-name" class="truncate text-sm font-semibold text-app-text">
                {{ authSessionState.user.display_name }}
              </span>
              <Tag
                v-if="authSessionState.user.is_admin"
                data-testid="sidebar-user-badge"
                :value="t('adminUsers.isAdmin')"
                severity="secondary"
                class="shrink-0 text-[0.68rem] font-semibold uppercase tracking-[0.08em]"
              />
            </div>
            <span data-testid="sidebar-user-login" class="truncate text-xs text-app-text-dim">
              @{{ authSessionState.user.login_name }}
            </span>
          </div>
          <Button
            v-if="isAuthenticated()"
            class="app-control-button app-sidebar-icon-button"
            type="button"
            :aria-label="t('auth.logout')"
            :title="t('auth.logout')"
            @click="signOut"
          >
            <AppMdiIcon :path="mdiLogoutVariant" :title="t('auth.logout')" class="app-sidebar-link-icon" />
          </Button>
        </div>
      </div>
    </aside>
  </div>
</template>
