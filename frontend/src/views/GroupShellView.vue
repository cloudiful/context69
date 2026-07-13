<script setup lang="ts">
import { provide, proxyRefs } from "vue";

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

    <UModal
      v-model:open="state.moveGroupDialogVisible"

      :title="$t('groups.moveGroup')"
      class="w-[30rem] max-w-[96vw]"
    >
    <template #body>
<div class="grid gap-3">
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ $t("groups.targetGroup") }}</label>
          <UInputMenu
            v-model="state.selectedTargetGroup"
            :items="state.groupSuggestions"
            label-key="name"
            :placeholder="$t('groups.selectTargetGroup')"
          >
            <template #item="{ item }">
              <div class="grid gap-0.5">
                <span>{{ item.name }}</span>
                <span class="text-sm text-muted">{{ item.group_key }}</span>
              </div>
            </template>
          </UInputMenu>
        </div>
      </div>
    </template>

      <template #footer>
        <div class="flex justify-end gap-2">
          <UButton color="neutral" variant="outline" @click="state.moveGroupDialogVisible = false">
            {{ $t("common.cancel") }}
          </UButton>
          <UButton :disabled="state.childGroupDialogBusy" @click="state.submitMoveGroup">
            {{ $t("common.move") }}
          </UButton>
        </div>
      </template>
    </UModal>
  </div>
</template>
