<script setup lang="ts">
import Button from "primevue/button";
import Message from "primevue/message";
import { useI18n } from "vue-i18n";

import AppPanel from "../components/AppPanel.vue";
import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import SettingsAdminUsersSection from "../components/settings-sections/SettingsAdminUsersSection.vue";
import SettingsDoclingSection from "../components/settings-sections/SettingsDoclingSection.vue";
import SettingsRuntimeSection from "../components/settings-sections/SettingsRuntimeSection.vue";
import SettingsSearchSection from "../components/settings-sections/SettingsSearchSection.vue";
import { useSettingsPage } from "../composables/use-settings-page";

const { t } = useI18n();

const {
  activeSectionId,
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
  enrichmentToggleModel,
  hasChanges,
  imageExportModeOptions,
  loading,
  ocrEngineOptions,
  ocrLangText,
  ocrToggleModel,
  pageError,
  pdfBackendOptions,
  pollPresetOptions,
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
  rerankApiKeyToggleModel,
  rerankToggleModel,
  resetAdminUserPassword,
  saveMessage,
  saveSettings,
  saving,
  schedulerToggleModel,
  searchHasStoredApiKey,
  scrollToSettingsSection,
  searchDraft,
  searchModeOptions,
  selectedPollPreset,
  selectedProviderAccount,
  selectedProviderAccountKey,
  selectedTimeoutPreset,
  settingsNavGroups,
  startNewProviderAccount,
  runtimeDraft,
  timeoutPresetOptions,
  toggleClearProviderApiKey,
  updateAdminUser,
} = useSettingsPage();
</script>

<template>
  <AppPanel class="settings-panel" :title="t('settings.title')">
    <template #actions>
      <div class="settings-header-actions">
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
        <div class="grid gap-2 xl:grid-cols-[minmax(10rem,11.5rem)_minmax(0,1fr)] xl:items-start xl:gap-x-6">
          <nav class="settings-anchor-nav" :aria-label="t('settings.navigationTitle')">
            <div class="settings-nav-groups">
              <section
                v-for="group in settingsNavGroups"
                :key="group.key"
                class="settings-nav-group"
              >
                <p class="settings-nav-group-title">{{ group.label }}</p>
                <div class="settings-nav-list">
                  <button
                    v-for="item in group.items"
                    :key="item.id"
                    type="button"
                    class="settings-nav-button"
                    :class="{ 'is-active': activeSectionId === item.id }"
                    :aria-current="activeSectionId === item.id ? 'location' : undefined"
                    @click="scrollToSettingsSection(item.id)"
                  >
                    {{ item.label }}
                  </button>
                </div>
              </section>
            </div>
          </nav>

          <div class="settings-sections">
            <SettingsSearchSection
              :clear-recent-searches="clearRecentSearches"
              :recent-search-count="recentSearchCount"
              :rerank-api-key-draft="rerankApiKeyDraft"
              :rerank-api-key-toggle-model="rerankApiKeyToggleModel"
              :rerank-toggle-model="rerankToggleModel"
              :search-has-stored-api-key="searchHasStoredApiKey"
              :search-draft="searchDraft"
              :search-mode-options="searchModeOptions"
              @update:rerank-api-key-draft="rerankApiKeyDraft = $event"
              @update:rerank-api-key-toggle-model="rerankApiKeyToggleModel = $event"
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
              :enrichment-toggle-model="enrichmentToggleModel"
              :image-export-mode-options="imageExportModeOptions"
              :ocr-engine-options="ocrEngineOptions"
              :ocr-lang-text="ocrLangText"
              :ocr-toggle-model="ocrToggleModel"
              :pdf-backend-options="pdfBackendOptions"
              :poll-preset-options="pollPresetOptions"
              :selected-poll-preset="selectedPollPreset"
              :selected-timeout-preset="selectedTimeoutPreset"
              :timeout-preset-options="timeoutPresetOptions"
              @update:enrichment-toggle-model="enrichmentToggleModel = $event"
              @update:ocr-lang-text="ocrLangText = $event"
              @update:ocr-toggle-model="ocrToggleModel = $event"
              @update:selected-poll-preset="selectedPollPreset = $event"
              @update:selected-timeout-preset="selectedTimeoutPreset = $event"
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
