<script setup lang="ts">
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Tag from "./AppTag.vue";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { authSessionState, isAuthenticated, logout } from "../services/auth/session";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { controlButtonClass } from "../ui/button-classes";

const props = withDefaults(defineProps<{
  collapsed?: boolean;
}>(), {
  collapsed: false,
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

function isItemActive(item: (typeof items.value)[number]) {
  return isActive(item.activePrefix ?? item.to);
}

function hasActiveChildren(item: (typeof items.value)[number]) {
  return !!item.children?.length && isItemActive(item);
}

function handleItemNavigate(item: (typeof items.value)[number]) {
  if (props.collapsed && item.children?.length) {
    preferences.expandSidebar();
  }
}

async function signOut() {
  await logout();
  await router.replace({
    name: "login",
  });
}
</script>

<template>
  <nav class="grid flex-1 content-start gap-1 overflow-y-auto py-3">
    <div
      v-for="item in isAuthenticated() ? items : []"
      :key="item.to"
      class="grid gap-1"
    >
      <RouterLink
        :to="item.to"
        :class="[
          'group flex items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm transition',
          isItemActive(item) ? 'is-active bg-app-surface-soft/70 text-app-text' : 'text-app-text-muted hover:bg-app-surface-soft/40 hover:text-app-text',
        ]"
        :data-nav-key="item.activePrefix ?? item.to"
        :title="item.label"
        @click="handleItemNavigate(item)"
      >
        <span :class="[
          'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border transition-colors',
          isItemActive(item) ? 'border-app-border-strong bg-app-surface-soft text-app-text' : 'border-app-border/60 bg-app-surface-soft/45 text-app-text-dim group-hover:text-app-text',
        ]">
          <AppMdiIcon :path="item.iconPath" :title="item.label" class="app-sidebar-link-icon h-4 w-4" />
        </span>
        <Transition name="sidebar-label">
          <span v-if="!collapsed">{{ item.label }}</span>
        </Transition>
      </RouterLink>

      <div
        v-if="!collapsed && hasActiveChildren(item)"
        class="ml-4 grid gap-1 pl-3"
      >
        <RouterLink
          v-for="child in item.children"
          :key="child.to"
          :to="child.to"
          :class="[
            'flex min-w-0 items-center rounded-lg px-2.5 py-1.5 text-xs transition',
            isActive(child.to) ? 'is-active bg-app-surface-soft/55 text-app-text' : 'text-app-text-dim hover:bg-app-surface-soft/35 hover:text-app-text',
          ]"
          :data-nav-child-key="child.to"
        >
          <span>{{ child.label }}</span>
        </RouterLink>
      </div>
    </div>
  </nav>

  <div class="app-sidebar-footer mt-auto shrink-0 border-t border-app-border/50 pt-3">
    <div
      class="flex items-end gap-2"
      :class="{ 'justify-center': collapsed }"
    >
      <Transition name="sidebar-user-card">
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
      </Transition>
      <Button
        v-if="isAuthenticated()"
        :class="[controlButtonClass, 'inline-flex h-9 w-9 shrink-0 items-center justify-center px-0']"
        type="button"
        :aria-label="t('auth.logout')"
        :title="t('auth.logout')"
        @click="signOut"
      >
        <AppMdiIcon :path="mdiLogoutVariant" :title="t('auth.logout')" class="app-sidebar-link-icon h-4 w-4" />
      </Button>
    </div>
  </div>
</template>
