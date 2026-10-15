<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";
import { buildSearchReturnLocation, isSearchReturn } from "../../utils/search-target";
import { loadSearchSession } from "../../utils/search";

const state = useGroupWorkspaceContext();
const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const fromSearch = computed(() => isSearchReturn(route.query));
const groupName = computed(() => state.group?.name ?? null);

function goBackToGroups() {
  void router.push({ name: "groups" });
}

function goBackToSearch() {
  if (window.history.length > 1) {
    router.back();
    return;
  }
  const session = loadSearchSession();
  if (session?.filters.query) {
    void router.push(buildSearchReturnLocation(session.filters, session.page));
    return;
  }
  void router.push({ name: "search" });
}
</script>

<template>
  <div class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
    <div class="flex shrink-0 flex-wrap items-center justify-start gap-2">
      <UButton
        data-testid="group-back-to-groups"
        color="neutral"
        variant="ghost"
        size="sm"
        icon="i-lucide-list"
        class="shrink-0"
        @click="goBackToGroups"
      >
        {{ t("library.backToGroups") }}
      </UButton>
      <UButton
        v-if="fromSearch"
        data-testid="group-back-to-search"
        color="neutral"
        variant="ghost"
        size="sm"
        icon="i-lucide-arrow-left"
        class="shrink-0"
        @click="goBackToSearch"
      >
        {{ t("document.backToSearch") }}
      </UButton>
    </div>
    <ProjectFilesPanel
    :child-groups="state.childGroups"
    :child-group-page="state.childrenPage"
    :child-group-search="state.childrenSearch"
    :group-path="state.groupPath"
    :group-name="groupName"
    @create-child-group="state.openCreateChildGroupDialog"
    @delete-child-group="state.confirmDeleteChildGroup"
    @edit-child-group="state.openEditChildGroupDialog"
    @move-child-group="state.openMoveChildGroupDialog"
    @open-child-group="state.openGroup"
    @child-group-page="state.changeChildrenPage"
    @child-group-page-size="state.changePageSize"
    @update:child-group-search="state.childrenSearch = $event"
    />
  </div>
</template>
