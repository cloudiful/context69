<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import InputText from "primevue/inputtext";
import Select from "primevue/select";
import type { GroupMemberResponse, MembershipRole, UpsertMembershipRequest } from "../services/api";
import { settingsDangerButtonClass, toolPrimaryButtonClass } from "../ui/button-classes";

const props = defineProps<{
  busy?: boolean;
  members: GroupMemberResponse[];
  title: string;
}>();

const emit = defineEmits<{
  add: [UpsertMembershipRequest];
  remove: [string];
}>();

const { t } = useI18n();
const loginName = ref("");
const role = ref<MembershipRole>("viewer");

const roleOptions = computed(() => [
  { label: "owner", value: "owner" },
  { label: "maintainer", value: "maintainer" },
  { label: "viewer", value: "viewer" },
]);

function submit() {
  const value = loginName.value.trim();
  if (!value) return;
  emit("add", { login_name: value, role: role.value });
  loginName.value = "";
  role.value = "viewer";
}
</script>

<template>
  <section class="grid gap-3 rounded-[1.1rem] border border-surface bg-emphasis p-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <p class="text-base font-semibold text-color">{{ title }}</p>
      </div>
    </div>

    <form class="grid gap-2 lg:grid-cols-[repeat(4,minmax(0,1fr))_auto] lg:items-center" @submit.prevent="submit">
      <InputText v-model="loginName" :placeholder="t('members.loginName')" />
      <Select v-model="role" :options="roleOptions" option-label="label" option-value="value" />
      <Button :class="toolPrimaryButtonClass" type="submit" :disabled="busy">
        {{ t("members.add") }}
      </Button>
    </form>

    <div class="grid gap-2">
      <div v-for="member in members" :key="member.user_id" class="flex flex-wrap items-center justify-between gap-3">
        <div class="grid gap-0.5 text-sm">
          <strong>{{ member.display_name }}</strong>
          <span class="text-muted-color">{{ member.login_name }} · {{ member.role }}</span>
        </div>
        <Button :class="settingsDangerButtonClass" :disabled="busy" @click="emit('remove', member.login_name)">
          {{ t("common.delete") }}
        </Button>
      </div>
    </div>
  </section>
</template>
