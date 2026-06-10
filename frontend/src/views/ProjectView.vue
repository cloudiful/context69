<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import Message from "primevue/message";
import AutoComplete from "primevue/autocomplete";
import Tag from "primevue/tag";
import { useConfirm } from "primevue/useconfirm";

import AppPanel from "../components/AppPanel.vue";
import EntityDialog from "../components/EntityDialog.vue";
import MemberDialog from "../components/MemberDialog.vue";
import ProjectFilesPanel from "../components/ProjectFilesPanel.vue";
import ProjectSourcesPanel from "../components/ProjectSourcesPanel.vue";
import {
  apiClient,
  type GroupResponse,
  type ProjectMemberResponse,
  type ProjectResponse,
  type UserDirectoryEntryResponse,
} from "../services/api";

type ProjectTab = "overview" | "sources" | "files" | "members";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const confirm = useConfirm();

const groupKey = computed(() => String(route.params.groupKey ?? ""));
const projectKey = computed(() => String(route.params.projectKey ?? ""));
const project = ref<ProjectResponse | null>(null);
const members = ref<ProjectMemberResponse[]>([]);
const groups = ref<GroupResponse[]>([]);
const errorMessage = ref("");
const memberError = ref("");
const actionError = ref("");
const activeTab = ref<ProjectTab>("overview");

const projectDialogVisible = ref(false);
const moveDialogVisible = ref(false);
const memberDialogVisible = ref(false);
const actionBusy = ref(false);
const editingMember = ref<ProjectMemberResponse | null>(null);
const memberSuggestions = ref<UserDirectoryEntryResponse[]>([]);
const selectedMemberUser = ref<UserDirectoryEntryResponse | null>(null);
const selectedTargetGroup = ref<GroupResponse | null>(null);

const tabs = computed(() => [
  { key: "overview", label: t("project.tabs.overview") },
  { key: "sources", label: t("project.tabs.sources") },
  { key: "files", label: t("project.tabs.files") },
  { key: "members", label: t("project.tabs.members") },
]);

function roleRank(role?: string | null) {
  if (role === "owner") return 3;
  if (role === "maintainer") return 2;
  if (role === "viewer") return 1;
  return 0;
}

const canManageProject = computed(() => roleRank(project.value?.current_role) >= 2);
const canOwnProject = computed(() => roleRank(project.value?.current_role) >= 3);

async function loadProject() {
  try {
    errorMessage.value = "";
    const [nextProject, nextMembers, nextGroups] = await Promise.all([
      apiClient.getProject(groupKey.value, projectKey.value),
      apiClient.listProjectMembers(groupKey.value, projectKey.value),
      apiClient.listGroups(),
    ]);
    project.value = nextProject;
    members.value = nextMembers;
    groups.value = nextGroups.filter((item) => item.group_key !== groupKey.value);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : t("project.loadFailed");
  }
}

async function searchUsers(query: string) {
  memberSuggestions.value = await apiClient.searchUserDirectory(query, 10);
}

async function saveProject(payload: { name: string; visibility: "private" | "public" }) {
  actionBusy.value = true;
  actionError.value = "";
  try {
    await apiClient.updateProject(groupKey.value, projectKey.value, {
      name: payload.name,
      visibility: payload.visibility,
    });
    projectDialogVisible.value = false;
    await loadProject();
  } catch (error) {
    actionError.value = error instanceof Error ? error.message : t("project.updateFailed");
  } finally {
    actionBusy.value = false;
  }
}

async function moveProject() {
  if (!selectedTargetGroup.value) return;
  actionBusy.value = true;
  actionError.value = "";
  try {
    const moved = await apiClient.moveProject(groupKey.value, projectKey.value, {
      target_group_key: selectedTargetGroup.value.group_key,
    });
    moveDialogVisible.value = false;
    selectedTargetGroup.value = null;
    void router.push({
      name: "project",
      params: { groupKey: moved.group_key, projectKey: moved.project_key },
    });
  } catch (error) {
    actionError.value = error instanceof Error ? error.message : t("project.moveFailed");
  } finally {
    actionBusy.value = false;
  }
}

async function saveMember(payload: { login_name: string; role: "owner" | "maintainer" | "viewer" }) {
  actionBusy.value = true;
  memberError.value = "";
  try {
    await apiClient.upsertProjectMember(groupKey.value, projectKey.value, payload);
    memberDialogVisible.value = false;
    editingMember.value = null;
    selectedMemberUser.value = null;
    members.value = await apiClient.listProjectMembers(groupKey.value, projectKey.value);
  } catch (error) {
    memberError.value = error instanceof Error ? error.message : t("project.membersFailed");
  } finally {
    actionBusy.value = false;
  }
}

function confirmDeleteProject() {
  confirm.require({
    header: t("common.delete"),
    message: t("project.deleteConfirm", { name: project.value?.name ?? projectKey.value }),
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("common.delete"), severity: "danger" },
    accept: () => void deleteProject(),
  });
}

async function deleteProject() {
  await apiClient.deleteProject(groupKey.value, projectKey.value);
  void router.push({ name: "group-detail", params: { groupKey: groupKey.value } });
}

function confirmRemoveMember(member: ProjectMemberResponse) {
  confirm.require({
    header: t("common.delete"),
    message: t("project.memberRemoveConfirm", { loginName: member.login_name }),
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("common.delete"), severity: "danger" },
    accept: () => void removeMember(member.login_name),
  });
}

async function removeMember(loginName: string) {
  await apiClient.deleteProjectMember(groupKey.value, projectKey.value, loginName);
  members.value = await apiClient.listProjectMembers(groupKey.value, projectKey.value);
}

function roleSeverity(role?: string | null) {
  if (role === "owner") return "success";
  if (role === "maintainer") return "info";
  return "secondary";
}

function groupOptionLabel(option: GroupResponse) {
  return `${option.name} (${option.group_key})`;
}

onMounted(() => {
  void loadProject();
});
</script>

<template>
  <div class="workspace-page">
    <AppPanel :title="project?.name || projectKey">
      <template #actions>
        <div v-if="canManageProject" class="flex gap-2">
          <Button severity="secondary" variant="outlined" @click="projectDialogVisible = true">
            {{ t("common.edit") }}
          </Button>
          <Button v-if="canOwnProject" severity="secondary" variant="outlined" @click="moveDialogVisible = true">
            {{ t("common.move") }}
          </Button>
          <Button v-if="canOwnProject" severity="danger" variant="outlined" @click="confirmDeleteProject">
            {{ t("common.delete") }}
          </Button>
        </div>
      </template>

      <Message v-if="errorMessage" severity="error" :closable="false">{{ errorMessage }}</Message>
      <Message v-if="actionError" severity="error" :closable="false">{{ actionError }}</Message>

      <div class="workspace-tabbar">
        <Button
          v-for="tab in tabs"
          :key="tab.key"
          class="workspace-tab-button"
          :class="{ 'is-active': activeTab === tab.key }"
          type="button"
          severity="secondary"
          variant="text"
          @click="activeTab = tab.key as ProjectTab"
        >
          {{ tab.label }}
        </Button>
      </div>

      <section v-if="activeTab === 'overview'" class="workspace-block">
        <div class="workspace-overview-grid">
          <div class="workspace-overview-card">
            <span class="section-label">{{ t("project.summary.group") }}</span>
            <strong>{{ groupKey }}</strong>
          </div>
          <div class="workspace-overview-card">
            <span class="section-label">{{ t("project.summary.project") }}</span>
            <strong>{{ projectKey }}</strong>
          </div>
          <div class="workspace-overview-card">
            <span class="section-label">{{ t("project.summary.visibility") }}</span>
            <Tag :value="project?.visibility || '--'" severity="secondary" />
          </div>
          <div class="workspace-overview-card">
            <span class="section-label">{{ t("groups.currentRole") }}</span>
            <Tag :value="project?.current_role || '--'" :severity="roleSeverity(project?.current_role)" />
          </div>
          <div class="workspace-overview-card">
            <span class="section-label">{{ t("project.summary.members") }}</span>
            <strong>{{ members.length }}</strong>
          </div>
        </div>
      </section>

      <ProjectSourcesPanel
        v-else-if="activeTab === 'sources'"
        :group-key="groupKey"
        :project-key="projectKey"
        :can-manage="canManageProject"
      />
      <ProjectFilesPanel v-else-if="activeTab === 'files'" :group-key="groupKey" :project-key="projectKey" />

      <section v-else class="workspace-block">
        <div class="workspace-block-header">
          <div>
            <p class="section-title">{{ t("project.membersTitle") }}</p>
          </div>
          <Button
            v-if="canManageProject"
            class="tool-action-primary"
            @click="editingMember = null; selectedMemberUser = null; memberDialogVisible = true"
          >
            {{ t("members.add") }}
          </Button>
        </div>

        <Message v-if="memberError" severity="error" :closable="false">{{ memberError }}</Message>

        <DataTable :value="members" data-key="user_id" scrollable size="small" table-style="min-width: 100%">
          <Column field="login_name" :header="t('adminUsers.loginName')" />
          <Column field="display_name" :header="t('adminUsers.displayName')" />
          <Column field="role" :header="t('members.role')">
            <template #body="{ data }">
              <Tag :value="data.role" :severity="roleSeverity(data.role)" />
            </template>
          </Column>
          <Column v-if="canManageProject" :header="t('common.edit')">
            <template #body="{ data }">
              <div class="flex gap-2">
                <Button
                  severity="secondary"
                  variant="outlined"
                  size="small"
                  @click="editingMember = data; selectedMemberUser = { user_id: data.user_id, login_name: data.login_name, display_name: data.display_name }; memberDialogVisible = true"
                >
                  {{ t("common.edit") }}
                </Button>
                <Button severity="danger" variant="outlined" size="small" @click="confirmRemoveMember(data)">
                  {{ t("common.delete") }}
                </Button>
              </div>
            </template>
          </Column>
        </DataTable>
      </section>

      <EntityDialog
        v-model:visible="projectDialogVisible"
        :busy="actionBusy"
        :error="actionError"
        :title="t('project.editProject')"
        :entity-name-label="t('groups.projectName')"
        :initial-name="project?.name"
        :initial-visibility="(project?.visibility as 'private' | 'public' | undefined)"
        @submit="saveProject"
      />

      <MemberDialog
        v-model:visible="memberDialogVisible"
        :busy="actionBusy"
        :error="memberError"
        :title="editingMember ? t('members.editTitle') : t('members.addTitle')"
        :selected-user="selectedMemberUser"
        :initial-login-name="editingMember?.login_name"
        :initial-role="editingMember?.role"
        :allow-user-search="!editingMember"
        :suggestions="memberSuggestions"
        @search-users="searchUsers"
        @update:selected-user="selectedMemberUser = $event"
        @submit="saveMember"
      />

      <Dialog
        v-model:visible="moveDialogVisible"
        modal
        :header="t('project.moveProject')"
        :style="{ width: '30rem', maxWidth: '96vw' }"
      >
        <div class="grid gap-3">
          <div class="grid gap-2">
            <label class="form-label">{{ t("groups.targetGroup") }}</label>
            <AutoComplete
              v-model="selectedTargetGroup"
              fluid
              dropdown
              force-selection
              :suggestions="groups"
              :option-label="groupOptionLabel"
              :placeholder="t('groups.selectTargetGroup')"
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
            <Button severity="secondary" variant="outlined" @click="moveDialogVisible = false">
              {{ t("common.cancel") }}
            </Button>
            <Button :disabled="actionBusy || !selectedTargetGroup" @click="moveProject">
              {{ t("common.move") }}
            </Button>
          </div>
        </template>
      </Dialog>
    </AppPanel>
  </div>
</template>
