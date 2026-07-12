<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Breadcrumb from "primevue/breadcrumb";
import Button from "primevue/button";

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

function groupPathCrumbs(path: string, currentIsGroup: boolean): Crumb[] {
  const segments = path.split("/").filter(Boolean);
  return segments.map((segment, index) => {
    const cumulativePath = segments.slice(0, index + 1).join("/");
    const isCurrent = currentIsGroup && index === segments.length - 1;
    return {
      label: segment,
      to: isCurrent ? undefined : `/groups/${encodeURIComponent(cumulativePath)}`,
    };
  });
}

const items = computed<Crumb[]>(() => {
  if (routeName.value === "search") {
    return [{ label: t("nav.search") }];
  }

  if (routeName.value === "groups") {
    return [{ label: t("nav.groups") }];
  }

  if (routeName.value.startsWith("group-")) {
    const isOverview = routeName.value === "group-overview";
    const crumbs: Crumb[] = [
      { label: t("nav.groups"), to: "/groups" },
      ...groupPathCrumbs(groupPath.value, isOverview),
    ];

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
  <div class="flex min-w-0 items-center gap-3">
    <Breadcrumb
      v-if="items.length > 1"
      :model="items"
      class="min-w-0 flex-1"
    >
      <template #item="{ item }">
        <Button
          v-if="item.to"
          class="min-w-0 max-w-full justify-start px-0"
          type="button"
          size="small"
          severity="secondary"
          text
          @click="navigate(item.to)"
        >
          <span class="truncate">{{ item.label }}</span>
        </Button>
        <span v-else class="truncate text-sm font-semibold text-color">
          {{ item.label }}
        </span>
      </template>
    </Breadcrumb>
    <div id="app-route-actions" class="flex shrink-0 items-center justify-end" />
  </div>
</template>
