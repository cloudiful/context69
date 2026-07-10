<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Password from "primevue/password";
import Tag from "primevue/tag";
import ToggleSwitch from "primevue/toggleswitch";
import { useConfirm } from "primevue/useconfirm";

import { appFormDialogPt } from "../app-dialog";
import type { AdminUserResponse } from "../../services/api";

const props = defineProps<{
  busy?: boolean;
  createBusy?: boolean;
  users: AdminUserResponse[];
}>();

const emit = defineEmits<{
  create: [{ login_name: string; display_name: string; password: string; is_admin: boolean }];
  resetPassword: [{ login_name: string; password: string }];
  update: [{ login_name: string; display_name: string; is_admin: boolean }];
  disable: [string];
  enable: [string];
}>();

const { t } = useI18n();
const confirm = useConfirm();

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
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("adminUsers.disableUser"), severity: "danger" },
    accept: () => emit("disable", loginNameValue),
  });
}

function confirmEnable(loginNameValue: string) {
  confirm.require({
    header: t("adminUsers.enableUser"),
    message: t("adminUsers.enableConfirm", { loginName: loginNameValue }),
    icon: "pi pi-check-circle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("adminUsers.enableUser") },
    accept: () => emit("enable", loginNameValue),
  });
}
</script>

<template>
  <section id="settings-admin-users" class="settings-block">
    <div class="settings-block-header">
      <p class="settings-block-title">{{ t("adminUsers.title") }}</p>
      <Button :label="t('adminUsers.create')" size="small" :disabled="createBusy" @click="openCreate" />
    </div>

    <DataTable
      class="app-data-table"
      :value="users"
      data-key="user_id"
      resizable-columns
      column-resize-mode="expand"
      scrollable
      state-storage="local"
      state-key="context69:table:admin-users:v2"
      table-style="min-width: 100%"
    >
      <Column field="login_name" :header="t('adminUsers.loginName')" sortable header-class="whitespace-nowrap" body-class="whitespace-nowrap" />
      <Column field="display_name" :header="t('adminUsers.displayName')" sortable header-class="whitespace-nowrap" body-class="whitespace-nowrap" />
      <Column :header="t('adminUsers.isAdmin')" header-class="whitespace-nowrap" body-class="whitespace-nowrap">
        <template #body="{ data }">
          <Tag
            class="whitespace-nowrap"
            :value="data.is_admin ? t('common.yes') : t('common.no')"
            :severity="data.is_admin ? 'success' : 'secondary'"
          />
        </template>
      </Column>
      <Column :header="t('adminUsers.status')" header-class="whitespace-nowrap" body-class="whitespace-nowrap">
        <template #body="{ data }">
          <Tag
            class="whitespace-nowrap"
            :value="statusLabel(data)"
            :severity="data.disabled_at ? 'warn' : 'success'"
          />
        </template>
      </Column>
      <Column field="created_at" :header="t('adminUsers.createdAt')" sortable header-class="whitespace-nowrap" body-class="whitespace-nowrap" />
      <Column :header="t('common.edit')" header-class="whitespace-nowrap" body-class="whitespace-nowrap">
        <template #body="{ data }">
          <div class="flex flex-nowrap items-center gap-2 whitespace-nowrap">
            <Button :label="t('common.edit')" severity="secondary" variant="outlined" size="small" @click="openEdit(data)" />
            <Button :label="t('adminUsers.resetPasswordAction')" severity="secondary" variant="outlined" size="small" @click="openReset(data)" />
            <Button
              v-if="!data.disabled_at"
              :label="t('adminUsers.disableUser')"
              severity="danger"
              variant="outlined"
              size="small"
              @click="confirmDisable(data.login_name)"
            />
            <Button
              v-else
              :label="t('adminUsers.enableUser')"
              severity="secondary"
              variant="outlined"
              size="small"
              @click="confirmEnable(data.login_name)"
            />
          </div>
        </template>
      </Column>
    </DataTable>

    <Dialog
      v-model:visible="createDialogVisible"
      modal
      :header="t('adminUsers.create')"
      :pt="appFormDialogPt"
      class="w-[30rem] max-w-[96vw]"
    >
      <div class="grid gap-3">
        <div class="grid gap-2">
          <label class="form-label">{{ t("adminUsers.loginName") }}</label>
          <InputText v-model="loginName" :placeholder="t('adminUsers.loginName')" />
        </div>
        <div class="grid gap-2">
          <label class="form-label">{{ t("adminUsers.displayName") }}</label>
          <InputText v-model="displayName" :placeholder="t('adminUsers.displayName')" />
        </div>
        <div class="grid gap-2">
          <label class="form-label">{{ t("adminUsers.password") }}</label>
          <Password v-model="password" fluid :feedback="false" toggle-mask />
        </div>
        <label class="flex items-center gap-2 text-sm text-app-text">
          <span>{{ t("adminUsers.isAdmin") }}</span>
          <ToggleSwitch v-model="isAdmin" />
        </label>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button severity="secondary" variant="outlined" @click="createDialogVisible = false">
            {{ t("common.cancel") }}
          </Button>
          <Button :disabled="createBusy || !loginName.trim() || !displayName.trim() || !password.trim()" @click="submitCreate">
            {{ t("adminUsers.create") }}
          </Button>
        </div>
      </template>
    </Dialog>

    <Dialog
      v-model:visible="editDialogVisible"
      modal
      :header="t('common.edit')"
      :pt="appFormDialogPt"
      class="w-[28rem] max-w-[96vw]"
    >
      <div class="grid gap-3">
        <div class="grid gap-2">
          <label class="form-label">{{ t("adminUsers.displayName") }}</label>
          <InputText v-model="displayName" :placeholder="t('adminUsers.displayName')" />
        </div>
        <label class="flex items-center gap-2 text-sm text-app-text">
          <span>{{ t("adminUsers.isAdmin") }}</span>
          <ToggleSwitch v-model="isAdmin" />
        </label>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button severity="secondary" variant="outlined" @click="editDialogVisible = false">
            {{ t("common.cancel") }}
          </Button>
          <Button :disabled="busy || !displayName.trim()" @click="submitEdit">
            {{ t("common.save") }}
          </Button>
        </div>
      </template>
    </Dialog>

    <Dialog
      v-model:visible="resetDialogVisible"
      modal
      :header="t('adminUsers.resetPasswordAction')"
      :pt="appFormDialogPt"
      class="w-[28rem] max-w-[96vw]"
    >
      <div class="grid gap-3">
        <div class="grid gap-2">
          <label class="form-label">{{ t("adminUsers.resetPassword") }}</label>
          <Password v-model="password" fluid :feedback="false" toggle-mask />
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button severity="secondary" variant="outlined" @click="resetDialogVisible = false">
            {{ t("common.cancel") }}
          </Button>
          <Button :disabled="busy || !password.trim()" @click="submitReset">
            {{ t("adminUsers.resetPasswordAction") }}
          </Button>
        </div>
      </template>
    </Dialog>
  </section>
</template>
