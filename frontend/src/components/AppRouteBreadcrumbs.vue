<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Breadcrumb from "primevue/breadcrumb";

import { resolveSettingsSectionNav } from "../settings/navigation";
import { authSessionState } from "../services/auth/session";

type Crumb = {
  label: string;
  to?: string;
};

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const routeName = computed(() => String(route.name ?? ""));
const groupPath = computed(() => String(route.params.groupPath ?? ""));

const settingsSections = computed(() => resolveSettingsSectionNav(t, authSessionState.user?.is_admin === true));

const items = computed<Crumb[]>(() => {
  if (routeName.value === "search") {
    return [{ label: t("nav.search") }];
  }

  if (routeName.value === "groups") {
    return [{ label: t("nav.groups") }];
  }

  if (routeName.value.startsWith("group-")) {
    const groupHome = `/groups/${encodeURIComponent(groupPath.value)}`;
    const crumbs: Crumb[] = routeName.value === "group-overview"
      ? [{ label: t("nav.groups") }]
      : [{ label: t("nav.groups"), to: groupHome }];

    if (routeName.value === "group-members") {
      crumbs.push({ label: t("groups.membersTitle") });
    } else if (routeName.value === "group-settings") {
      crumbs.push({ label: t("nav.settings") });
    }

    return crumbs;
  }

  if (routeName.value.startsWith("settings-")) {
    const activeSection = settingsSections.value.find((item) => item.to === route.path);
    const crumbs: Crumb[] = [
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
  <div class="flex min-w-0 items-center gap-3 rounded-[1rem] border border-app-border/60 bg-app-surface-muted/18 px-3 py-2">
    <Breadcrumb
      v-if="items.length > 1"
      :model="items"
      class="min-w-0 flex-1 [&.p-breadcrumb]:border-0 [&.p-breadcrumb]:bg-transparent [&.p-breadcrumb]:p-0"
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
    <div id="app-route-actions" class="flex shrink-0 items-center justify-end" />
  </div>
</template>
