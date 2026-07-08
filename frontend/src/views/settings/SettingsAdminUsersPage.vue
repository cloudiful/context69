<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsAdminUsersSection from "../../components/settings-sections/SettingsAdminUsersSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const adminUsers = computed(() => unref(state.adminUsers));
const adminUsersBusy = computed(() => unref(state.adminUsersBusy));
const adminUsersCreateBusy = computed(() => unref(state.adminUsersCreateBusy));
const adminUsersError = computed(() => unref(state.adminUsersError));
</script>

<template>
  <SettingsAdminUsersSection
    v-if="adminUsers.length > 0 || adminUsersBusy || adminUsersError"
    :busy="adminUsersBusy"
    :create-busy="adminUsersCreateBusy"
    :error="adminUsersError"
    :users="adminUsers"
    @create="state.createAdminUser"
    @disable="state.disableAdminUser"
    @enable="state.enableAdminUser"
    @reset-password="state.resetAdminUserPassword"
    @update="state.updateAdminUser"
  />
</template>
