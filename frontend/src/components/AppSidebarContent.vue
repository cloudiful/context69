<script setup lang="ts">
import { RouterLink, useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import SidebarContent from "primevue/sidebarcontent";
import SidebarFooter from "primevue/sidebarfooter";
import SidebarMenu from "primevue/sidebarmenu";
import SidebarMenuButton from "primevue/sidebarmenubutton";
import SidebarMenuItem from "primevue/sidebarmenuitem";
import SidebarMenuSub from "primevue/sidebarmenusub";
import SidebarMenuSubButton from "primevue/sidebarmenusubbutton";
import SidebarMenuSubItem from "primevue/sidebarmenusubitem";
import Tag from "primevue/tag";

import AppMdiIcon from "./AppMdiIcon.vue";
import { useAppNavigation } from "../composables/use-app-navigation";
import { authSessionState, isAuthenticated, logout } from "../services/auth/session";
import { useUiPreferences } from "../composables/use-ui-preferences";

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
  <SidebarContent>
    <SidebarMenu>
      <SidebarMenuItem
      v-for="item in isAuthenticated() ? items : []"
      :key="item.to"
      >
        <SidebarMenuButton
        :as="RouterLink"
        :to="item.to"
        :is-active="isItemActive(item)"
        :data-nav-key="item.activePrefix ?? item.to"
        :title="item.label"
        @click="handleItemNavigate(item)"
      >
          <AppMdiIcon :path="item.iconPath" :title="item.label" />
          <span>{{ item.label }}</span>
        </SidebarMenuButton>

        <SidebarMenuSub v-if="hasActiveChildren(item)">
          <SidebarMenuSubItem
          v-for="child in item.children"
          :key="child.to"
          >
            <SidebarMenuSubButton
            :as="RouterLink"
            :to="child.to"
            :is-active="isActive(child.to)"
            :data-nav-child-key="child.to"
          >
              <span>{{ child.label }}</span>
            </SidebarMenuSubButton>
          </SidebarMenuSubItem>
        </SidebarMenuSub>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarContent>

  <SidebarFooter>
    <div
      class="flex items-end gap-2"
      :class="{ 'justify-center': collapsed }"
    >
      <div
        v-if="authSessionState.user && !collapsed"
        class="grid min-w-0 grow gap-1"
        data-testid="sidebar-user-card"
      >
        <div class="flex items-start justify-between gap-2">
          <span data-testid="sidebar-user-name" class="truncate text-sm font-semibold">
            {{ authSessionState.user.display_name }}
          </span>
          <Tag
            v-if="authSessionState.user.is_admin"
            data-testid="sidebar-user-badge"
            :value="t('adminUsers.isAdmin')"
            severity="secondary"
          />
        </div>
        <span data-testid="sidebar-user-login" class="truncate text-xs text-muted-color">
          @{{ authSessionState.user.login_name }}
        </span>
      </div>
      <Button
        v-if="isAuthenticated()"
        type="button"
        text
        rounded
        severity="secondary"
        :aria-label="t('auth.logout')"
        :title="t('auth.logout')"
        @click="signOut"
      >
        <AppMdiIcon :path="mdiLogoutVariant" :title="t('auth.logout')" />
      </Button>
    </div>
  </SidebarFooter>
</template>
