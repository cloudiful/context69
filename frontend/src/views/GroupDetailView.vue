<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Message from "primevue/message";
import AutoComplete from "primevue/autocomplete";
import Dialog from "primevue/dialog";
import Tag from "primevue/tag";
import { useConfirm } from "primevue/useconfirm";

import AppPanel from "../components/AppPanel.vue";
import EntityDialog from "../components/EntityDialog.vue";
import MemberDialog from "../components/MemberDialog.vue";
import {
  apiClient,
  type GroupMemberResponse,
  type GroupResponse,
  type ProjectResponse,
  type UserDirectoryEntryResponse,
} from "../services/api";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const confirm = useConfirm();

const groupKey = computed(() => String(route.params.groupKey ?? ""));
const group = ref<GroupResponse | null>(null);
const members = ref<GroupMemberResponse[]>([]);
const projects = ref<ProjectResponse[]>([]);
const errorMessage = ref("");
const memberError = ref("");
const projectError = ref("");
const loading = ref(false);

const groupDialogVisible = ref(false);
const groupDialogBusy = ref(false);
const projectDialogVisible = ref(false);
const projectDialogBusy = ref(false);
const moveProjectDialogVisible = ref(false);
const memberDialogVisible = ref(false);
const memberDialogBusy = ref(false);

const editingProject = ref<ProjectResponse | null>(null);
const movingProject = ref<ProjectResponse | null>(null);
const editingMember = ref<GroupMemberResponse | null>(null);
const memberSuggestions = ref<UserDirectoryEntryResponse[]>([]);
const selectedMemberUser = ref<UserDirectoryEntryResponse | null>(null);
const groupSuggestions = ref<GroupResponse[]>([]);
const selectedTargetGroup = ref<GroupResponse | null>(null);

async function loadPage() {
  loading.value = true;
  try {
    errorMessage.value = "";
    const [nextGroup, nextMembers, nextProjects, nextGroups] = await Promise.all([
      apiClient.getGroup(groupKey.value),
      apiClient.listGroupMembers(groupKey.value),
      apiClient.listProjects(groupKey.value),
      apiClient.listGroups(),
    ]);
    group.value = nextGroup;
    members.value = nextMembers;
    projects.value = nextProjects;
    groupSuggestions.value = nextGroups.filter((item) => item.group_key !== groupKey.value);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : t("groups.loadFailed");
  } finally {
    loading.value = false;
  }
}

async function searchUsers(query: string) {
  memberSuggestions.value = await apiClient.searchUserDirectory(query, 10);
}

async function saveGroup(payload: { name: string; visibility: "private" | "public" }) {
  groupDialogBusy.value = true;
  errorMessage.value = "";
  try {
    await apiClient.updateGroup(groupKey.value, {
      name: payload.name,
      visibility: payload.visibility,
    });
    groupDialogVisible.value = false;
    await loadPage();
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : t("groups.createFailed");
  } finally {
    groupDialogBusy.value = false;
  }
}

async function saveProject(payload: { key?: string; name: string; visibility: "private" | "public" }) {
  projectDialogBusy.value = true;
  projectError.value = "";
  try {
    if (editingProject.value) {
      await apiClient.updateProject(groupKey.value, editingProject.value.project_key, {
        name: payload.name,
        visibility: payload.visibility,
      });
    } else {
      await apiClient.createProject(groupKey.value, {
        project_key: payload.key?.trim() ?? "",
        name: payload.name,
        visibility: payload.visibility,
      });
    }
    projectDialogVisible.value = false;
    editingProject.value = null;
    await loadPage();
  } catch (error) {
    projectError.value = error instanceof Error ? error.message : t("groups.projectCreateFailed");
  } finally {
    projectDialogBusy.value = false;
  }
}

async function saveMember(payload: { login_name: string; role: "owner" | "maintainer" | "viewer" }) {
  memberDialogBusy.value = true;
  memberError.value = "";
  try {
    await apiClient.upsertGroupMember(groupKey.value, payload);
    memberDialogVisible.value = false;
    editingMember.value = null;
    selectedMemberUser.value = null;
    members.value = await apiClient.listGroupMembers(groupKey.value);
  } catch (error) {
    memberError.value = error instanceof Error ? error.message : t("groups.membersFailed");
  } finally {
    memberDialogBusy.value = false;
  }
}

async function submitMoveProject() {
  if (!movingProject.value || !selectedTargetGroup.value) return;
  projectDialogBusy.value = true;
  projectError.value = "";
  try {
    const moved = await apiClient.moveProject(groupKey.value, movingProject.value.project_key, {
      target_group_key: selectedTargetGroup.value.group_key,
    });
    moveProjectDialogVisible.value = false;
    movingProject.value = null;
    selectedTargetGroup.value = null;
    await loadPage();
    void router.push({
      name: "project",
      params: {
        groupKey: moved.group_key,
        projectKey: moved.project_key,
      },
    });
  } catch (error) {
    projectError.value = error instanceof Error ? error.message : t("groups.projectMoveFailed");
  } finally {
    projectDialogBusy.value = false;
  }
}

function openProject(project: ProjectResponse) {
  void router.push({
    name: "project",
    params: { groupKey: groupKey.value, projectKey: project.project_key },
  });
}

function confirmDeleteGroup() {
  confirm.require({
    header: t("common.delete"),
    message: t("groups.deleteConfirm", { name: group.value?.name ?? groupKey.value }),
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("common.delete"), severity: "danger" },
    accept: () => void deleteGroup(),
  });
}

async function deleteGroup() {
  await apiClient.deleteGroup(groupKey.value);
  void router.push({ name: "groups" });
}

function confirmDeleteProject(project: ProjectResponse) {
  confirm.require({
    header: t("common.delete"),
    message: t("groups.projectDeleteConfirm", { name: project.name }),
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("common.delete"), severity: "danger" },
    accept: () => void deleteProject(project),
  });
}

async function deleteProject(project: ProjectResponse) {
  await apiClient.deleteProject(groupKey.value, project.project_key);
  await loadPage();
}

function confirmRemoveMember(member: GroupMemberResponse) {
  confirm.require({
    header: t("common.delete"),
    message: t("groups.memberRemoveConfirm", { loginName: member.login_name }),
    icon: "pi pi-exclamation-triangle",
    rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
    acceptProps: { label: t("common.delete"), severity: "danger" },
    accept: () => void removeMember(member.login_name),
  });
}

async function removeMember(loginName: string) {
  await apiClient.deleteGroupMember(groupKey.value, loginName);
  members.value = await apiClient.listGroupMembers(groupKey.value);
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
  void loadPage();
});
</script>

<template>
  <div class="workspace-page">
    <AppPanel :title="group?.name || groupKey">
      <template #actions>
        <div class="flex gap-2">
          <Button severity="secondary" variant="outlined" @click="groupDialogVisible = true">
            {{ t("common.edit") }}
          </Button>
          <Button severity="danger" variant="outlined" @click="confirmDeleteGroup">
            {{ t("common.delete") }}
          </Button>
        </div>
      </template>

      <Message v-if="errorMessage" severity="error" :closable="false">{{ errorMessage }}</Message>

      <section class="workspace-block">
        <div class="workspace-block-header">
          <div>
            <p class="section-title">{{ t("groups.projectsTitle") }}</p>
            <p class="workspace-muted">{{ group?.group_key }} · {{ group?.visibility }}</p>
          </div>
          <Button class="tool-action-primary" @click="projectDialogVisible = true">
            {{ t("groups.createProject") }}
          </Button>
        </div>

        <Message v-if="projectError" severity="error" :closable="false">{{ projectError }}</Message>

        <DataTable :value="projects" data-key="project_id" scrollable size="small" table-style="min-width: 100%">
          <Column field="project_key" :header="t('groups.projectKey')" />
          <Column field="name" :header="t('groups.projectName')" />
          <Column field="visibility" :header="t('groups.visibility')">
            <template #body="{ data }">
              <Tag :value="data.visibility" severity="secondary" />
            </template>
          </Column>
          <Column :header="t('common.open')">
            <template #body="{ data }">
              <div class="flex gap-2">
                <Button severity="secondary" variant="outlined" size="small" @click="openProject(data)">
                  {{ t("common.open") }}
                </Button>
                <Button severity="secondary" variant="outlined" size="small" @click="editingProject = data; projectDialogVisible = true">
                  {{ t("common.edit") }}
                </Button>
                <Button severity="secondary" variant="outlined" size="small" @click="movingProject = data; selectedTargetGroup = null; moveProjectDialogVisible = true">
                  {{ t("common.move") }}
                </Button>
                <Button severity="danger" variant="outlined" size="small" @click="confirmDeleteProject(data)">
                  {{ t("common.delete") }}
                </Button>
              </div>
            </template>
          </Column>
        </DataTable>
      </section>

      <section class="workspace-block">
        <div class="workspace-block-header">
          <div>
            <p class="section-title">{{ t("groups.membersTitle") }}</p>
          </div>
          <Button class="tool-action-primary" @click="editingMember = null; selectedMemberUser = null; memberDialogVisible = true">
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
          <Column :header="t('common.edit')">
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
        v-model:visible="groupDialogVisible"
        :busy="groupDialogBusy"
        :error="errorMessage"
        :title="t('groups.editGroup')"
        :entity-name-label="t('groups.groupName')"
        :initial-name="group?.name"
        :initial-visibility="(group?.visibility as 'private' | 'public' | undefined)"
        @submit="saveGroup"
      />

      <EntityDialog
        v-model:visible="projectDialogVisible"
        :busy="projectDialogBusy"
        :error="projectError"
        :title="editingProject ? t('groups.editProject') : t('groups.createProject')"
        :show-key="!editingProject"
        :entity-key-label="t('groups.projectKey')"
        :entity-name-label="t('groups.projectName')"
        :initial-key="editingProject?.project_key"
        :initial-name="editingProject?.name"
        :initial-visibility="(editingProject?.visibility as 'private' | 'public' | undefined)"
        :submit-label="editingProject ? t('common.save') : t('groups.createProject')"
        @submit="saveProject"
      />

      <MemberDialog
        v-model:visible="memberDialogVisible"
        :busy="memberDialogBusy"
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
        v-model:visible="moveProjectDialogVisible"
        modal
        :header="t('groups.moveProject')"
        :style="{ width: '30rem', maxWidth: '96vw' }"
      >
        <div class="grid gap-3">
          <Message v-if="projectError" severity="error" :closable="false">{{ projectError }}</Message>
          <div class="grid gap-2">
            <label class="form-label">{{ t("groups.targetGroup") }}</label>
            <AutoComplete
              v-model="selectedTargetGroup"
              fluid
              dropdown
              force-selection
              :suggestions="groupSuggestions"
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
            <Button severity="secondary" variant="outlined" @click="moveProjectDialogVisible = false">
              {{ t("common.cancel") }}
            </Button>
            <Button :disabled="projectDialogBusy || !selectedTargetGroup" @click="submitMoveProject">
              {{ t("common.move") }}
            </Button>
          </div>
        </template>
      </Dialog>
    </AppPanel>
  </div>
</template>
