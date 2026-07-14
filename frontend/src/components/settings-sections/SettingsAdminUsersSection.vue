<script setup lang="ts">
import { computed, ref } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import { useAppConfirm } from "../../composables/use-app-confirm";

import AppSettingsBlock from "../AppSettingsBlock.vue";

import type { AdminUserResponse } from "../../services/api";

const props = defineProps<{
  busy?: boolean;
  createBusy?: boolean;
  page: number;
  pageSize: number;
  query: string;
  total: number;
  users: AdminUserResponse[];
}>();

const emit = defineEmits<{
  create: [{ login_name: string; display_name: string; password: string; is_admin: boolean }];
  resetPassword: [{ login_name: string; password: string }];
  update: [{ login_name: string; display_name: string; is_admin: boolean }];
  disable: [string];
  enable: [string];
  "update:query": [string];
  page: [number];
}>();

const { t } = useI18n();
const confirm = useAppConfirm();

const createDialogVisible = ref(false);
const editDialogVisible = ref(false);
const resetDialogVisible = ref(false);
const loginName = ref("");
const displayName = ref("");
const password = ref("");
const isAdmin = ref(false);
const editingUser = ref<AdminUserResponse | null>(null);
const resetUser = ref<AdminUserResponse | null>(null);

const statusLabel = computed(() => (user: AdminUserResponse) => user.disabled_at ? t("adminUsers.disabled") : t("adminUsers.active"));
const columns = computed<TableColumn<AdminUserResponse>[]>(() => [
  { accessorKey: "login_name", header: t("adminUsers.loginName") },
  { accessorKey: "display_name", header: t("adminUsers.displayName") },
  { id: "is_admin", header: t("adminUsers.isAdmin") },
  { id: "status", header: t("adminUsers.status") },
  { accessorKey: "created_at", header: t("adminUsers.createdAt") },
  { id: "actions", header: t("common.edit") },
]);
function resetCreateForm() {
  loginName.value = "";
  displayName.value = "";
  password.value = "";
  isAdmin.value = false;
}

function openCreate() {
  resetCreateForm();
  createDialogVisible.value = true;
}

function openEdit(user: AdminUserResponse) {
  editingUser.value = user;
  displayName.value = user.display_name;
  isAdmin.value = user.is_admin;
  editDialogVisible.value = true;
}

function openReset(user: AdminUserResponse) {
  resetUser.value = user;
  password.value = "";
  resetDialogVisible.value = true;
}

function submitCreate() {
  emit("create", {
    login_name: loginName.value.trim(),
    display_name: displayName.value.trim(),
    password: password.value,
    is_admin: isAdmin.value,
  });
  createDialogVisible.value = false;
}

function submitEdit() {
  if (!editingUser.value) return;
  emit("update", {
    login_name: editingUser.value.login_name,
    display_name: displayName.value.trim(),
    is_admin: isAdmin.value,
  });
  editDialogVisible.value = false;
}

function submitReset() {
  if (!resetUser.value) return;
  emit("resetPassword", {
    login_name: resetUser.value.login_name,
    password: password.value,
  });
  resetDialogVisible.value = false;
}

function confirmDisable(loginNameValue: string) {
  confirm.require({
    header: t("adminUsers.disableUser"),
    message: t("adminUsers.disableConfirm", { loginName: loginNameValue }),
    rejectLabel: t("common.cancel"),
    acceptLabel: t("adminUsers.disableUser"),
    accept: () => emit("disable", loginNameValue),
  });
}

function confirmEnable(loginNameValue: string) {
  confirm.require({
    header: t("adminUsers.enableUser"),
    message: t("adminUsers.enableConfirm", { loginName: loginNameValue }),
    rejectLabel: t("common.cancel"),
    acceptLabel: t("adminUsers.enableUser") ,
    accept: () => emit("enable", loginNameValue),
  });
}
</script>

<template>
  <AppSettingsBlock id="settings-admin-users" compact :title="t('adminUsers.title')">
    <template #actions>
      <UInput :model-value="query" class="w-56" icon="i-lucide-search" :placeholder="t('adminUsers.loginName')" @update:model-value="emit('update:query', $event)" />
      <UButton size="sm" :disabled="createBusy" @click="openCreate">{{ t("adminUsers.create") }}</UButton>
    </template>

    <UTable
      class="min-w-0 max-w-full"
      :data="users"
      :columns="columns"
      :loading="busy"
    >
      <template #is_admin-cell="{ row }">
          <UBadge
            class="whitespace-nowrap"
            :label="row.original.is_admin ? t('common.yes') : t('common.no')"
            :color="row.original.is_admin ? 'success' : 'neutral'"
            variant="subtle"
          />
      </template>
      <template #status-cell="{ row }">
          <UBadge
            class="whitespace-nowrap"
            :label="statusLabel(row.original)"
            :color="row.original.disabled_at ? 'warning' : 'success'"
            variant="subtle"
          />
      </template>
      <template #actions-cell="{ row }">
          <div class="flex flex-nowrap items-center gap-2 whitespace-nowrap">
            <UButton color="neutral" variant="outline" size="sm" @click="openEdit(row.original)">{{ t("common.edit") }}</UButton>
            <UButton color="neutral" variant="outline" size="sm" @click="openReset(row.original)">{{ t("adminUsers.resetPasswordAction") }}</UButton>
            <UButton
              v-if="!row.original.disabled_at"
              color="error"
              variant="outline"
              size="sm"
              @click="confirmDisable(row.original.login_name)"
            >
              {{ t("adminUsers.disableUser") }}
            </UButton>
            <UButton
              v-else
              color="neutral"
              variant="outline"
              size="sm"
              @click="confirmEnable(row.original.login_name)"
            >
              {{ t("adminUsers.enableUser") }}
            </UButton>
          </div>
      </template>
    </UTable>

    <UPagination
      v-if="total > pageSize"
      :page="page"
      :items-per-page="pageSize"
      :total="total"
      class="justify-end"
      @update:page="emit('page', $event)"
    />

    <UModal
      v-model:open="createDialogVisible"

      :title="t('adminUsers.create')"
      class="w-[30rem] max-w-[96vw]"
    >
    <template #body>
<div class="grid gap-3">
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("adminUsers.loginName") }}</label>
          <UInput v-model="loginName" :placeholder="t('adminUsers.loginName')" />
        </div>
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("adminUsers.displayName") }}</label>
          <UInput v-model="displayName" :placeholder="t('adminUsers.displayName')" />
        </div>
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("adminUsers.password") }}</label>
          <UInput type="password" v-model="password" fluid :feedback="false" toggle-mask />
        </div>
        <label class="flex items-center gap-2 text-sm text-color">
          <span>{{ t("adminUsers.isAdmin") }}</span>
          <USwitch v-model="isAdmin" />
        </label>
      </div>
    </template>

      <template #footer>
        <div class="flex justify-end gap-2">
          <UButton color="neutral" variant="outline" @click="createDialogVisible = false">
            {{ t("common.cancel") }}
          </UButton>
          <UButton :disabled="createBusy || !loginName.trim() || !displayName.trim() || !password.trim()" @click="submitCreate">
            {{ t("adminUsers.create") }}
          </UButton>
        </div>
      </template>
    </UModal>

    <UModal
      v-model:open="editDialogVisible"

      :title="t('common.edit')"
      class="w-[28rem] max-w-[96vw]"
    >
    <template #body>
<div class="grid gap-3">
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("adminUsers.displayName") }}</label>
          <UInput v-model="displayName" :placeholder="t('adminUsers.displayName')" />
        </div>
        <label class="flex items-center gap-2 text-sm text-color">
          <span>{{ t("adminUsers.isAdmin") }}</span>
          <USwitch v-model="isAdmin" />
        </label>
      </div>
    </template>

      <template #footer>
        <div class="flex justify-end gap-2">
          <UButton color="neutral" variant="outline" @click="editDialogVisible = false">
            {{ t("common.cancel") }}
          </UButton>
          <UButton :disabled="busy || !displayName.trim()" @click="submitEdit">
            {{ t("common.save") }}
          </UButton>
        </div>
      </template>
    </UModal>

    <UModal
      v-model:open="resetDialogVisible"

      :title="t('adminUsers.resetPasswordAction')"
      class="w-[28rem] max-w-[96vw]"
    >
    <template #body>
<div class="grid gap-3">
        <div class="grid gap-2">
          <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("adminUsers.resetPassword") }}</label>
          <UInput type="password" v-model="password" fluid :feedback="false" toggle-mask />
        </div>
      </div>
    </template>

      <template #footer>
        <div class="flex justify-end gap-2">
          <UButton color="neutral" variant="outline" @click="resetDialogVisible = false">
            {{ t("common.cancel") }}
          </UButton>
          <UButton :disabled="busy || !password.trim()" @click="submitReset">
            {{ t("adminUsers.resetPasswordAction") }}
          </UButton>
        </div>
      </template>
    </UModal>
  </AppSettingsBlock>
</template>
