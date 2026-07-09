<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Breadcrumb from "primevue/breadcrumb";

import { resolveSettingsSectionNav } from "../settings/navigation";
import { authSessionState } from "../services/auth/session";
import { useWorkspaceNavigationContext } from "../composables/use-workspace-navigation-context";

type Crumb = {
  label: string;
  to?: string;
};

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const workspaceContext = useWorkspaceNavigationContext();

const routeName = computed(() => String(route.name ?? ""));
const groupKey = computed(() => String(route.params.groupKey ?? ""));
const projectKey = computed(() => String(route.params.projectKey ?? ""));

const settingsSections = computed(() => resolveSettingsSectionNav(t, authSessionState.user?.is_admin === true));

const currentGroupLabel = computed(() => {
  if (workspaceContext.groupKey === groupKey.value && workspaceContext.groupLabel) {
    return workspaceContext.groupLabel;
  }
  return groupKey.value;
});

const currentProjectLabel = computed(() => {
  if (
    workspaceContext.projectGroupKey === groupKey.value
    && workspaceContext.projectKey === projectKey.value
    && workspaceContext.projectLabel
  ) {
    return workspaceContext.projectLabel;
  }
  return projectKey.value;
});

const items = computed<Crumb[]>(() => {
  if (routeName.value === "search") {
    return [{ label: t("nav.search") }];
  }

  if (routeName.value === "groups") {
    return [
      { label: t("nav.search"), to: "/search" },
      { label: t("nav.groups") },
    ];
  }

  if (routeName.value.startsWith("group-") || routeName.value === "group-detail") {
    const crumbs: Crumb[] = [
      { label: t("nav.search"), to: "/search" },
      { label: t("nav.groups"), to: "/groups" },
      { label: currentGroupLabel.value, to: `/groups/${groupKey.value}/overview` },
    ];

    if (routeName.value === "group-members") {
      crumbs.push({ label: t("groups.membersTitle") });
    } else if (routeName.value === "group-settings") {
      crumbs.push({ label: t("nav.settings") });
    }

    return crumbs;
  }

  if (routeName.value.startsWith("project-") || routeName.value === "project") {
    const crumbs: Crumb[] = [
      { label: t("nav.search"), to: "/search" },
      { label: t("nav.groups"), to: "/groups" },
      { label: currentGroupLabel.value, to: `/groups/${groupKey.value}/overview` },
      { label: currentProjectLabel.value, to: `/groups/${groupKey.value}/projects/${projectKey.value}/overview` },
    ];

    if (routeName.value === "project-members") {
      crumbs.push({ label: t("project.tabs.members") });
    } else if (routeName.value === "project-settings") {
      crumbs.push({ label: t("nav.settings") });
    }

    return crumbs;
  }

  if (routeName.value.startsWith("settings-")) {
    const activeSection = settingsSections.value.find((item) => item.to === route.path);
    const crumbs: Crumb[] = [
      { label: t("nav.search"), to: "/search" },
      { label: t("nav.settings"), to: "/settings/appearance" },
    ];
    if (activeSection && activeSection.label !== t("nav.settings")) {
      crumbs.push({ label: activeSection.label });
    }
    return crumbs;
  }

  return [];
});

function navigate(to?: string) {
  if (!to || to === route.path) {
    return;
  }
  void router.push(to);
}
</script>

<template>
  <div class="min-w-0 rounded-[1rem] border border-app-border/60 bg-app-surface-muted/18 px-3 py-2">
    <Breadcrumb
      :model="items"
      class="min-w-0 [&.p-breadcrumb]:border-0 [&.p-breadcrumb]:bg-transparent [&.p-breadcrumb]:p-0"
    >
      <template #item="{ item }">
        <button
          v-if="item.to"
          class="truncate text-left text-sm font-medium text-app-text transition hover:text-app-text-muted"
          type="button"
          @click="navigate(item.to)"
        >
          {{ item.label }}
        </button>
        <span v-else class="truncate text-sm font-semibold text-app-text">
          {{ item.label }}
        </span>
      </template>
    </Breadcrumb>
  </div>
</template>
