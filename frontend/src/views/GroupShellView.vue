<script setup lang="ts">
import { provide, proxyRefs } from "vue";
import AutoComplete from "primevue/autocomplete";
import Button from "primevue/button";
import Dialog from "primevue/dialog";

import EntityDialog from "../components/EntityDialog.vue";
import MemberDialog from "../components/MemberDialog.vue";
import { groupWorkspaceStateKey } from "../composables/group-workspace-context";
import { useGroupWorkspace } from "../composables/use-group-workspace";

const state = proxyRefs(useGroupWorkspace());

provide(groupWorkspaceStateKey, state);
</script>

<template>
  <div class="grid h-full min-h-0 grid-rows-[minmax(0,1fr)] gap-2">
    <RouterView />

    <EntityDialog
      v-model:visible="state.groupDialogVisible"
      :busy="state.groupDialogBusy"
      :title="$t('groups.editGroup')"
      :entity-name-label="$t('groups.groupName')"
      :initial-name="state.group?.name"
      :initial-visibility="(state.group?.visibility as 'private' | 'public' | undefined)"
      @submit="state.saveGroup"
    />

    <EntityDialog
      v-model:visible="state.childGroupDialogVisible"
      :busy="state.childGroupDialogBusy"
      :title="state.editingChildGroup ? $t('groups.editChild') : $t('groups.createChild')"
      :show-key="!state.editingChildGroup"
      :entity-key-label="$t('groups.groupKey')"
      :entity-name-label="$t('groups.groupName')"
      :initial-key="state.editingChildGroup?.group_key"
      :initial-name="state.editingChildGroup?.name"
      :initial-visibility="(state.editingChildGroup?.visibility as 'private' | 'public' | undefined)"
      :submit-label="state.editingChildGroup ? $t('common.save') : $t('groups.createChild')"
      @submit="state.saveChildGroup"
    />

    <MemberDialog
      v-model:visible="state.memberDialogVisible"
      :busy="state.memberDialogBusy"
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
      v-model:visible="state.moveGroupDialogVisible"
      modal
      :header="$t('groups.moveGroup')"
      class="w-[30rem] max-w-[96vw]"
    >
      <div class="grid gap-3">
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ $t("groups.targetGroup") }}</label>
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
                <span class="text-sm text-(--p-text-muted-color)">{{ option.group_key }}</span>
              </div>
            </template>
          </AutoComplete>
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button severity="secondary" variant="outlined" @click="state.moveGroupDialogVisible = false">
            {{ $t("common.cancel") }}
          </Button>
          <Button :disabled="state.childGroupDialogBusy" @click="state.submitMoveGroup">
            {{ $t("common.move") }}
          </Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
