<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import AutoComplete from "primevue/autocomplete";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Message from "primevue/message";
import Select from "primevue/select";

import { appFormDialogPt } from "./app-dialog";
import type { UserDirectoryEntryResponse } from "../services/api";

type MembershipRole = "owner" | "maintainer" | "viewer";

const props = defineProps<{
  visible: boolean;
  busy?: boolean;
  error?: string;
  title: string;
  initialLoginName?: string;
  initialRole?: MembershipRole;
  allowUserSearch?: boolean;
  selectedUser?: UserDirectoryEntryResponse | null;
  suggestions?: UserDirectoryEntryResponse[];
}>();

const emit = defineEmits<{
  "update:visible": [boolean];
  "search-users": [string];
  submit: [{ login_name: string; role: MembershipRole }];
  "update:selected-user": [UserDirectoryEntryResponse | null];
}>();

const { t } = useI18n();
const role = ref<MembershipRole>("viewer");
const manualLoginName = ref("");

const roleOptions = [
  { label: "owner", value: "owner" },
  { label: "maintainer", value: "maintainer" },
  { label: "viewer", value: "viewer" },
];

watch(
  () => props.visible,
  (visible) => {
    if (!visible) return;
    role.value = props.initialRole ?? "viewer";
    manualLoginName.value = props.initialLoginName ?? "";
  },
  { immediate: true },
);

watch(
  () => props.initialRole,
  (value) => {
    if (props.visible && value) {
      role.value = value;
    }
  },
);

watch(
  () => props.initialLoginName,
  (value) => {
    if (props.visible && value) {
      manualLoginName.value = value;
    }
  },
);

const selectedUserModel = computed<UserDirectoryEntryResponse | null>({
  get: () => props.selectedUser ?? null,
  set: (value) => emit("update:selected-user", value),
});

const canSubmit = computed(() => {
  if (props.allowUserSearch === false) {
    return manualLoginName.value.trim().length > 0;
  }
  return !!selectedUserModel.value?.login_name;
});

function close() {
  emit("update:visible", false);
}

function handleSubmit() {
  const loginName = props.allowUserSearch === false
    ? manualLoginName.value.trim()
    : selectedUserModel.value?.login_name?.trim() ?? "";
  if (!loginName) return;
  emit("submit", { login_name: loginName, role: role.value });
}

function userOptionLabel(option: UserDirectoryEntryResponse) {
  return `${option.display_name} (${option.login_name})`;
}
</script>

<template>
  <Dialog
    :visible="visible"
    modal
    :header="title"
    :pt="appFormDialogPt"
    :style="{ width: '30rem', maxWidth: '96vw' }"
    @update:visible="emit('update:visible', $event)"
  >
    <div class="grid gap-3">
      <Message v-if="error" severity="error" :closable="false">{{ error }}</Message>

      <div v-if="allowUserSearch !== false" class="grid gap-2">
        <label class="form-label">{{ t("members.user") }}</label>
        <AutoComplete
          v-model="selectedUserModel"
          fluid
          dropdown
          force-selection
          :suggestions="suggestions"
          :placeholder="t('members.searchUserPlaceholder')"
          @complete="emit('search-users', $event.query)"
        >
          <template #option="{ option }">
            <div class="grid gap-0.5">
              <span>{{ option.display_name }}</span>
              <span class="text-sm text-app-text-dim">{{ option.login_name }}</span>
            </div>
          </template>
        </AutoComplete>
      </div>

      <div v-else class="grid gap-2">
        <label class="form-label">{{ t("members.loginName") }}</label>
        <InputText
          v-model="manualLoginName"
          :placeholder="t('members.loginName')"
        />
      </div>

      <div class="grid gap-2">
        <label class="form-label">{{ t("members.role") }}</label>
        <Select
          v-model="role"
          fluid
          option-label="label"
          option-value="value"
          :options="roleOptions"
        />
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <Button severity="secondary" variant="outlined" :disabled="busy" @click="close">
          {{ t("common.cancel") }}
        </Button>
        <Button :disabled="busy || !canSubmit" @click="handleSubmit">
          {{ t("common.save") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
