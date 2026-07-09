<script setup lang="ts">
import { provide, proxyRefs } from "vue";
import AutoComplete from "primevue/autocomplete";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import Message from "primevue/message";

import EntityDialog from "../components/EntityDialog.vue";
import MemberDialog from "../components/MemberDialog.vue";
import { appFormDialogPt } from "../components/app-dialog";
import { projectWorkspaceStateKey } from "../composables/project-workspace-context";
import { useProjectWorkspace } from "../composables/use-project-workspace";

const state = proxyRefs(useProjectWorkspace());

provide(projectWorkspaceStateKey, state);
</script>

<template>
  <div class="workspace-page">
    <Message v-if="state.errorMessage" severity="error" :closable="false">{{ state.errorMessage }}</Message>
    <Message v-if="state.actionError" severity="error" :closable="false">{{ state.actionError }}</Message>

    <RouterView />

    <EntityDialog
      v-model:visible="state.projectDialogVisible"
      :busy="state.actionBusy"
      :error="state.actionError"
      :title="$t('project.editProject')"
      :entity-name-label="$t('groups.projectName')"
      :initial-name="state.project?.name"
      :initial-visibility="(state.project?.visibility as 'private' | 'public' | undefined)"
      @submit="state.saveProject"
    />

    <MemberDialog
      v-model:visible="state.memberDialogVisible"
      :busy="state.actionBusy"
      :error="state.memberError"
      :title="state.editingMember ? $t('members.editTitle') : $t('members.addTitle')"
      :selected-user="state.selectedMemberUser"
      :initial-login-name="state.editingMember?.login_name"
      :initial-role="state.editingMember?.role"
      :allow-user-search="!state.editingMember"
      :suggestions="state.memberSuggestions"
      @search-users="state.searchUsers"
      @update:selected-user="state.selectedMemberUser = $event"
      @submit="state.saveMember"
    />

    <Dialog
      v-model:visible="state.moveDialogVisible"
      modal
      :header="$t('project.moveProject')"
      :pt="appFormDialogPt"
      :style="{ width: '30rem', maxWidth: '96vw' }"
    >
      <div class="grid gap-3">
        <div class="grid gap-2">
          <label class="form-label">{{ $t("groups.targetGroup") }}</label>
          <AutoComplete
            v-model="state.selectedTargetGroup"
            fluid
            dropdown
            force-selection
            :suggestions="state.groups"
            :option-label="state.groupOptionLabel"
            :placeholder="$t('groups.selectTargetGroup')"
          >
            <template #option="{ option }">
              <div class="grid gap-0.5">
                <span>{{ option.name }}</span>
                <span class="text-sm text-app-text-dim">{{ option.group_key }}</span>
              </div>
            </template>
          </AutoComplete>
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button severity="secondary" variant="outlined" @click="state.moveDialogVisible = false">
            {{ $t("common.cancel") }}
          </Button>
          <Button :disabled="state.actionBusy || !state.selectedTargetGroup" @click="state.moveProject">
            {{ $t("common.move") }}
          </Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
