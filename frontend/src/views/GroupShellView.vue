<script setup lang="ts">
import { provide, proxyRefs } from "vue";
import AutoComplete from "primevue/autocomplete";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import Message from "primevue/message";

import EntityDialog from "../components/EntityDialog.vue";
import MemberDialog from "../components/MemberDialog.vue";
import { appFormDialogPt } from "../components/app-dialog";
import { groupWorkspaceStateKey } from "../composables/group-workspace-context";
import { useGroupWorkspace } from "../composables/use-group-workspace";

const state = proxyRefs(useGroupWorkspace());

provide(groupWorkspaceStateKey, state);
</script>

<template>
  <div class="workspace-page">
    <Message v-if="state.errorMessage" severity="error" :closable="false">{{ state.errorMessage }}</Message>

    <RouterView />

    <EntityDialog
      v-model:visible="state.groupDialogVisible"
      :busy="state.groupDialogBusy"
      :error="state.errorMessage"
      :title="$t('groups.editGroup')"
      :entity-name-label="$t('groups.groupName')"
      :initial-name="state.group?.name"
      :initial-visibility="(state.group?.visibility as 'private' | 'public' | undefined)"
      @submit="state.saveGroup"
    />

    <EntityDialog
      v-model:visible="state.projectDialogVisible"
      :busy="state.projectDialogBusy"
      :error="state.projectError"
      :title="state.editingProject ? $t('groups.editProject') : $t('groups.createProject')"
      :show-key="!state.editingProject"
      :entity-key-label="$t('groups.projectKey')"
      :entity-name-label="$t('groups.projectName')"
      :initial-key="state.editingProject?.project_key"
      :initial-name="state.editingProject?.name"
      :initial-visibility="(state.editingProject?.visibility as 'private' | 'public' | undefined)"
      :submit-label="state.editingProject ? $t('common.save') : $t('groups.createProject')"
      @submit="state.saveProject"
    />

    <MemberDialog
      v-model:visible="state.memberDialogVisible"
      :busy="state.memberDialogBusy"
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
      v-model:visible="state.moveProjectDialogVisible"
      modal
      :header="$t('groups.moveProject')"
      :pt="appFormDialogPt"
      :style="{ width: '30rem', maxWidth: '96vw' }"
    >
      <div class="grid gap-3">
        <Message v-if="state.projectError" severity="error" :closable="false">{{ state.projectError }}</Message>
        <div class="grid gap-2">
          <label class="form-label">{{ $t("groups.targetGroup") }}</label>
          <AutoComplete
            v-model="state.selectedTargetGroup"
            fluid
            dropdown
            force-selection
            :suggestions="state.groupSuggestions"
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
          <Button severity="secondary" variant="outlined" @click="state.moveProjectDialogVisible = false">
            {{ $t("common.cancel") }}
          </Button>
          <Button :disabled="state.projectDialogBusy || !state.selectedTargetGroup" @click="state.submitMoveProject">
            {{ $t("common.move") }}
          </Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
