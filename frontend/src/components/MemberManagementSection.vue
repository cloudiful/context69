<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import InputText from "primevue/inputtext";
import Message from "primevue/message";
import Select from "primevue/select";

type MemberRow = {
  user_id: number;
  login_name: string;
  display_name: string;
  role: "owner" | "maintainer" | "viewer";
};

const props = defineProps<{
  busy?: boolean;
  error?: string;
  members: MemberRow[];
  title: string;
}>();

const emit = defineEmits<{
  add: [{ login_name: string; role: "owner" | "maintainer" | "viewer" }];
  remove: [string];
}>();

const { t } = useI18n();
const loginName = ref("");
const role = ref<"owner" | "maintainer" | "viewer">("viewer");

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
  <section class="workspace-block">
    <div class="workspace-block-header">
      <div>
        <p class="section-title">{{ title }}</p>
      </div>
    </div>

    <Message v-if="error" severity="error" :closable="false">{{ error }}</Message>

    <form class="workspace-inline-form" @submit.prevent="submit">
      <InputText v-model="loginName" :placeholder="t('members.loginName')" />
      <Select v-model="role" :options="roleOptions" option-label="label" option-value="value" />
      <Button type="submit" :disabled="busy">
        {{ t("members.add") }}
      </Button>
    </form>

    <div class="workspace-list">
      <div v-for="member in members" :key="member.user_id" class="workspace-list-row">
        <div class="workspace-list-copy">
          <strong>{{ member.display_name }}</strong>
          <span>{{ member.login_name }} · {{ member.role }}</span>
        </div>
        <Button severity="danger" variant="outlined" :disabled="busy" @click="emit('remove', member.login_name)">
          {{ t("common.delete") }}
        </Button>
      </div>
    </div>
  </section>
</template>
