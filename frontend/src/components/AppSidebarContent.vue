<script setup lang="ts">
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Tag from "primevue/tag";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { authSessionState, isAuthenticated, logout } from "../services/auth";
import { useUiPreferences } from "../composables/use-ui-preferences";

const props = withDefaults(defineProps<{
  collapsed?: boolean;
  closeOnNavigate?: boolean;
}>(), {
  collapsed: false,
  closeOnNavigate: false,
});

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const preferences = useUiPreferences();
const items = useAppNavigation();
const mdiLogoutVariant = "M14.08,15.59L16.67,13H7V11H16.67L14.08,8.41L15.5,7L20.5,12L15.5,17L14.08,15.59M4,19H10V21H4C2.89,21 2,20.1 2,19V5C2,3.89 2.89,3 4,3H10V5H4V19Z";

function isActive(itemTo: string) {
  return route.path === itemTo || route.path.startsWith(`${itemTo}/`);
}

function handleNavigate() {
  if (props.closeOnNavigate) {
    preferences.closeMobileNav();
  }
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
  <div class="flex h-full flex-col">
    <nav class="grid flex-1 content-start gap-1 overflow-y-auto py-3">
      <RouterLink
        v-for="item in isAuthenticated() ? items : []"
        :key="item.to"
        :to="item.to"
        class="flex items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm text-app-text-muted transition hover:bg-app-surface-soft/40 hover:text-app-text"
        :class="{ 'is-active bg-app-surface-soft/70 text-app-text': isActive(item.to) }"
        :data-nav-key="item.to"
        :title="item.label"
        @click="handleNavigate"
      >
        <span
          class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-app-border/60 bg-app-surface-soft/45 text-app-text-dim transition-colors"
          :class="{ 'border-app-border-strong bg-app-surface-soft text-app-text': isActive(item.to) }"
        >
          <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon h-4 w-4" />
        </span>
        <span v-if="!collapsed">{{ item.label }}</span>
      </RouterLink>
    </nav>

    <div class="app-sidebar-footer mt-auto shrink-0 border-t border-app-border/50 pt-3">
      <div
        class="flex items-end gap-2"
        :class="{ 'justify-center': collapsed }"
      >
        <div
          v-if="authSessionState.user && !collapsed"
          class="grid grow gap-1 rounded-xl border border-app-border/70 bg-app-surface-muted/35 px-3 py-2.5"
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
          class="inline-flex h-9 w-9 shrink-0 items-center justify-center px-0"
          type="button"
          severity="secondary"
          variant="outlined"
          :aria-label="t('auth.logout')"
          :title="t('auth.logout')"
          @click="signOut"
        >
          <AppMdiIcon :path="mdiLogoutVariant" :title="t('auth.logout')" class="app-sidebar-link-icon h-4 w-4" />
        </Button>
      </div>
    </div>
  </div>
</template>
