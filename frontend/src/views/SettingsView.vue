<script setup lang="ts">
import Button from "primevue/button";
import Message from "primevue/message";
import { useI18n } from "vue-i18n";

import AppMdiIcon from "../components/AppMdiIcon.vue";
import AppPanel from "../components/AppPanel.vue";
import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import SettingsAdminUsersSection from "../components/settings-sections/SettingsAdminUsersSection.vue";
import SettingsAppearanceSection from "../components/settings-sections/SettingsAppearanceSection.vue";
import SettingsDoclingSection from "../components/settings-sections/SettingsDoclingSection.vue";
import SettingsRuntimeSection from "../components/settings-sections/SettingsRuntimeSection.vue";
import SettingsSearchSection from "../components/settings-sections/SettingsSearchSection.vue";
import { persistLocale, type AppLocale } from "../i18n/locale";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { useSettingsPage } from "../composables/use-settings-page";

const { t, locale } = useI18n({ useScope: "global" });
const preferences = useUiPreferences();
const mdiMenu = "M3,6H21V8H3V6M3,11H21V13H3V11M3,16H21V18H3V16Z";
const appLocale = locale as { value: AppLocale };

const {
  adminUsers,
  adminUsersBusy,
  adminUsersCreateBusy,
  adminUsersError,
  clearRecentSearches,
  createAdminUser,
  deleteProviderAccount,
  doclingDraft,
  disableAdminUser,
  doclingProviderOptions,
  enableAdminUser,
  hasChanges,
  loading,
  pageError,
  providerAccountOptions,
  providerDraft,
  providerKindOptions,
  providerMessage,
  providerSaving,
  providerStatusLabel,
  providerToggleModel,
  qdrantToggleModel,
  recentSearchCount,
  rerankApiKeyDraft,
  rerankToggleModel,
  resetAdminUserPassword,
  saveMessage,
  saveSettings,
  saving,
  schedulerToggleModel,
  searchHasStoredApiKey,
  searchDraft,
  searchModeOptions,
  selectedProviderAccount,
  selectedProviderAccountKey,
  startNewProviderAccount,
  runtimeDraft,
  toggleClearProviderApiKey,
  updateAdminUser,
} = useSettingsPage();

function switchLocale(nextLocale: AppLocale) {
  if (locale.value === nextLocale) {
    return;
  }

  locale.value = nextLocale;
  persistLocale(nextLocale);
}
</script>

<template>
  <AppPanel class="settings-panel">
    <template #actions>
      <div class="settings-header-actions">
        <Button
          class="app-control-button md:hidden"
          type="button"
          :aria-label="t('settings.openNavigation')"
          @click="preferences.toggleMobileNav"
        >
          <AppMdiIcon :path="mdiMenu" :title="t('settings.openNavigation')" class="app-sidebar-link-icon" />
        </Button>
        <Message v-if="hasChanges" severity="secondary" :closable="false">
          {{ t("settings.status.pending") }}
        </Message>
        <Message v-if="saveMessage" severity="success" :closable="false">
          {{ saveMessage }}
        </Message>
      </div>
    </template>

    <AsyncStateBlock
      :loading="loading"
      :loading-title="t('settings.loadingTitle')"
      :loading-message="t('settings.loadingMessage')"
      :error="pageError"
    >
      <form class="grid gap-2" @submit.prevent="saveSettings">
        <div class="settings-sections">
          <SettingsAppearanceSection
            :locale="appLocale.value"
            :theme="preferences.state.theme"
            @update:locale="switchLocale"
            @update:theme="preferences.setTheme"
          />

          <SettingsSearchSection
            :clear-recent-searches="clearRecentSearches"
            :recent-search-count="recentSearchCount"
            :rerank-api-key-draft="rerankApiKeyDraft"
            :rerank-toggle-model="rerankToggleModel"
            :search-has-stored-api-key="searchHasStoredApiKey"
            :search-draft="searchDraft"
            :search-mode-options="searchModeOptions"
            @update:rerank-api-key-draft="rerankApiKeyDraft = $event"
            @update:rerank-toggle-model="rerankToggleModel = $event"
          />

          <SettingsRuntimeSection
            :delete-provider-account="deleteProviderAccount"
            :provider-account-options="providerAccountOptions"
            :provider-draft="providerDraft"
            :provider-kind-options="providerKindOptions"
            :provider-message="providerMessage"
            :provider-saving="providerSaving"
            :provider-status-label="providerStatusLabel"
            :provider-toggle-model="providerToggleModel"
            :qdrant-toggle-model="qdrantToggleModel"
            :runtime-draft="runtimeDraft"
            :saving="saving"
            :scheduler-toggle-model="schedulerToggleModel"
            :selected-provider-account="selectedProviderAccount"
            :selected-provider-account-key="selectedProviderAccountKey"
            :start-new-provider-account="startNewProviderAccount"
            :toggle-clear-provider-api-key="toggleClearProviderApiKey"
            @update:provider-toggle-model="providerToggleModel = $event"
            @update:qdrant-toggle-model="qdrantToggleModel = $event"
            @update:scheduler-toggle-model="schedulerToggleModel = $event"
            @update:selected-provider-account-key="selectedProviderAccountKey = $event"
          />

          <SettingsDoclingSection
            :docling-draft="doclingDraft"
            :docling-provider-options="doclingProviderOptions"
          />

          <SettingsAdminUsersSection
            v-if="adminUsers.length > 0 || adminUsersBusy || adminUsersError"
            :busy="adminUsersBusy"
            :create-busy="adminUsersCreateBusy"
            :error="adminUsersError"
            :users="adminUsers"
            @create="createAdminUser"
            @disable="disableAdminUser"
            @enable="enableAdminUser"
            @reset-password="resetAdminUserPassword"
            @update="updateAdminUser"
          />
        </div>

        <div class="settings-save-bar">
          <Button
            data-testid="settings-save"
            type="submit"
            :disabled="saving || providerSaving || !hasChanges"
            :label="saving ? t('common.loading') : t('common.save')"
          />
        </div>
      </form>
    </AsyncStateBlock>
  </AppPanel>
</template>
