<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsAdminUsersSection from "../../components/settings-sections/SettingsAdminUsersSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const adminUsers = computed(() => unref(state.adminUsers));
const adminUsersBusy = computed(() => unref(state.adminUsersBusy));
const adminUsersCreateBusy = computed(() => unref(state.adminUsersCreateBusy));
const adminUsersPage = computed(() => unref(state.adminUsersPage));
const adminUsersQuery = computed(() => unref(state.adminUsersQuery));
</script>

<template>
  <SettingsAdminUsersSection
    :busy="adminUsersBusy"
    :create-busy="adminUsersCreateBusy"
    :pagination="adminUsersPage.pagination"
    :query="adminUsersQuery"
    :users="adminUsers"
    @create="state.createAdminUser"
    @disable="state.disableAdminUser"
    @enable="state.enableAdminUser"
    @reset-password="state.resetAdminUserPassword"
    @update="state.updateAdminUser"
    @update:query="state.adminUsersQuery.value = $event"
    @page="state.changeAdminUsersPage"
    @page-size="state.changeAdminUsersPageSize"
    @sort-change="state.changeAdminUsersSort"
  />
</template>
