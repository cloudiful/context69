<script setup lang="ts">
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Message from "primevue/message";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <section class="workspace-block">
    <div class="workspace-block-header">
      <div>
        <p class="section-title">{{ $t("groups.projectsTitle") }}</p>
      </div>
      <Button :label="$t('groups.createProject')" size="small" @click="state.openCreateProjectDialog" />
    </div>

    <Message v-if="state.projectError" severity="error" :closable="false">{{ state.projectError }}</Message>

    <DataTable class="app-data-table" :value="state.projects" data-key="project_id" scrollable table-style="min-width: 100%">
      <Column field="project_key" :header="$t('groups.projectKey')" />
      <Column field="name" :header="$t('groups.projectName')" />
      <Column field="visibility" :header="$t('groups.visibility')">
        <template #body="{ data }">
          <Tag :value="data.visibility" severity="secondary" />
        </template>
      </Column>
      <Column :header="$t('common.open')">
        <template #body="{ data }">
          <div class="flex gap-2">
            <Button severity="secondary" variant="outlined" size="small" @click="state.openProject(data)">
              {{ $t("common.open") }}
            </Button>
            <Button severity="secondary" variant="outlined" size="small" @click="state.openEditProjectDialog(data)">
              {{ $t("common.edit") }}
            </Button>
            <Button severity="secondary" variant="outlined" size="small" @click="state.openMoveProjectDialog(data)">
              {{ $t("common.move") }}
            </Button>
            <Button severity="danger" variant="outlined" size="small" @click="state.confirmDeleteProject(data)">
              {{ $t("common.delete") }}
            </Button>
          </div>
        </template>
      </Column>
    </DataTable>
  </section>
</template>
