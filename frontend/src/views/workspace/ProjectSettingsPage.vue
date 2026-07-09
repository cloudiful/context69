<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import InputText from "primevue/inputtext";
import Select from "primevue/select";
import Tag from "primevue/tag";

import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
const { t } = useI18n();

const projectName = ref("");
const visibility = ref<"private" | "public">("private");

const visibilityOptions = computed(() => [
  { label: t("groups.visibilityOptions.private"), value: "private" },
  { label: t("groups.visibilityOptions.public"), value: "public" },
]);

const canSave = computed(() => {
  if (!state.canManageProject) {
    return false;
  }

  const nextName = projectName.value.trim();
  if (!nextName) {
    return false;
  }

  return nextName !== (state.project?.name ?? "") || visibility.value !== (state.project?.visibility ?? "private");
});

watch(
  () => state.project,
  (project) => {
    projectName.value = project?.name ?? "";
    visibility.value = (project?.visibility as "private" | "public" | undefined) ?? "private";
  },
  { immediate: true },
);

async function saveProjectSettings() {
  const name = projectName.value.trim();
  if (!name) {
    return;
  }

  await state.saveProject({
    name,
    visibility: visibility.value,
  });
}
</script>

<template>
  <div class="workspace-settings-layout">
    <section class="workspace-summary-card">
      <dl class="workspace-summary-list">
        <div class="workspace-summary-row">
          <dt class="workspace-summary-label">{{ $t("project.summary.group") }}</dt>
          <dd class="workspace-summary-value">{{ state.groupKey }}</dd>
        </div>
        <div class="workspace-summary-row">
          <dt class="workspace-summary-label">{{ $t("project.summary.project") }}</dt>
          <dd class="workspace-summary-value">{{ state.projectKey }}</dd>
        </div>
        <div class="workspace-summary-row">
          <dt class="workspace-summary-label">{{ $t("groups.projectName") }}</dt>
          <dd class="workspace-summary-value">
            <InputText
              v-if="state.canManageProject"
              v-model="projectName"
              class="w-full min-w-0 text-right"
            />
            <span v-else>{{ state.project?.name || "--" }}</span>
          </dd>
        </div>
        <div class="workspace-summary-row">
          <dt class="workspace-summary-label">{{ $t("project.summary.visibility") }}</dt>
          <dd class="workspace-summary-value">
            <Select
              v-if="state.canManageProject"
              v-model="visibility"
              class="min-w-[8rem] text-left"
              option-label="label"
              option-value="value"
              :options="visibilityOptions"
            />
            <Tag v-else :value="state.project?.visibility || '--'" severity="secondary" />
          </dd>
        </div>
        <div class="workspace-summary-row">
          <dt class="workspace-summary-label">{{ $t("groups.currentRole") }}</dt>
          <dd class="workspace-summary-value">
            <Tag :value="state.project?.current_role || '--'" :severity="state.roleSeverity(state.project?.current_role)" />
          </dd>
        </div>
      </dl>

      <div v-if="state.canManageProject" class="flex justify-end">
        <Button :disabled="state.actionBusy || !canSave" @click="saveProjectSettings">
          {{ $t("common.save") }}
        </Button>
      </div>
    </section>

    <section v-if="state.canOwnProject" class="workspace-summary-card">
      <Button severity="danger" variant="outlined" @click="state.confirmDeleteProject">
        {{ $t("common.delete") }}
      </Button>
    </section>
  </div>
</template>
