<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

import { useAppNavigation } from "../composables/use-app-navigation";
import { authSessionState, isAuthenticated, logout } from "../services/auth/session";

const props = withDefaults(defineProps<{ collapsed?: boolean }>(), { collapsed: false });
const router = useRouter();
const { t } = useI18n();
const items = useAppNavigation();

const navigationItems = computed(() => isAuthenticated()
  ? items.value.map((item) => {
    const active = router.currentRoute.value.path === item.to
      || router.currentRoute.value.path.startsWith(`${item.activePrefix ?? item.to}/`);

    return {
      label: item.label,
      icon: item.icon,
      to: item.to,
      active,
      defaultOpen: active && !!item.children?.length,
      children: item.children?.map((child) => ({ label: child.label, to: child.to })),
    };
  })
  : []);

async function signOut() {
  await logout();
  await router.replace({ name: "login" });
}
</script>

<template>
  <UNavigationMenu
    orientation="vertical"
    :collapsed="props.collapsed"
    :items="navigationItems"
    class="min-h-0 flex-1"
  />

  <template v-if="authSessionState.user">
    <div v-if="!props.collapsed" class="mt-auto grid min-w-0 gap-1 border-t border-default pt-3" data-testid="sidebar-user-card">
      <div class="flex items-start justify-between gap-2">
        <span data-testid="sidebar-user-name" class="truncate text-sm font-semibold">
          {{ authSessionState.user.display_name }}
        </span>
        <UBadge v-if="authSessionState.user.is_admin" data-testid="sidebar-user-badge" :label="t('adminUsers.isAdmin')" color="neutral" variant="subtle" />
      </div>
      <span data-testid="sidebar-user-login" class="truncate text-xs text-muted">@{{ authSessionState.user.login_name }}</span>
    </div>
    <UButton
      class="mt-2"
      :block="!props.collapsed"
      color="neutral"
      variant="ghost"
      icon="i-lucide-log-out"
      :label="props.collapsed ? undefined : t('auth.logout')"
      :aria-label="t('auth.logout')"
      :title="t('auth.logout')"
      @click="signOut"
    />
  </template>
</template>
