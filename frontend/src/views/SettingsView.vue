<script setup lang="ts">
import { computed, provide, unref } from "vue";
import { useRoute } from "vue-router";
import Button from "primevue/button";
import Message from "primevue/message";
import { useI18n } from "vue-i18n";

import AppPanel from "../components/AppPanel.vue";
import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import { settingsPageStateKey } from "../composables/settings-page-context";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { useSettingsPage } from "../composables/use-settings-page";
import { normalizeAppLocale, persistLocale, type AppLocale } from "../i18n/locale";
import type { SettingsSectionKey } from "../settings/navigation";
import { settingsFloatingSaveButtonClass } from "../ui/button-classes";
import SettingsAccessTokensPage from "./settings/SettingsAccessTokensPage.vue";
import SettingsAdminUsersPage from "./settings/SettingsAdminUsersPage.vue";
import SettingsAppearancePage from "./settings/SettingsAppearancePage.vue";
import SettingsDoclingPage from "./settings/SettingsDoclingPage.vue";
import SettingsRuntimePage from "./settings/SettingsRuntimePage.vue";
import SettingsSearchPage from "./settings/SettingsSearchPage.vue";

const { t, locale } = useI18n({ useScope: "global" });
const route = useRoute();
const preferences = useUiPreferences();
const state = useSettingsPage();

provide(settingsPageStateKey, state);

const currentSection = computed<SettingsSectionKey>(() => {
  switch (route.name) {
    case "settings-access-tokens":
      return "access-tokens";
    case "settings-search":
      return "search";
    case "settings-runtime":
      return "runtime";
    case "settings-docling":
      return "docling";
    case "settings-admin-users":
      return "admin-users";
    case "settings-appearance":
    default:
      return "appearance";
  }
});

const hasChanges = computed(() => unref(state.hasChanges));
const loading = computed(() => unref(state.loading));
const pageError = computed(() => unref(state.pageError));
const providerSaving = computed(() => unref(state.providerSaving));
const saveMessage = computed(() => unref(state.saveMessage));
const saving = computed(() => unref(state.saving));
const currentLocale = computed<AppLocale>(() => normalizeAppLocale(String(locale.value)) ?? "en");

function switchLocale(nextLocale: AppLocale) {
  if (currentLocale.value === nextLocale) {
    return;
  }

  locale.value = nextLocale;
  persistLocale(nextLocale);
}
</script>

<template>
  <AppPanel surface="plain" class="settings-panel">
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
      <form class="grid gap-2" @submit.prevent="state.saveSettings">
        <div class="grid gap-4">
          <SettingsAppearancePage
            v-if="currentSection === 'appearance'"
            :locale="currentLocale"
            :theme="preferences.state.theme"
            @update:locale="switchLocale"
            @update:theme="preferences.setTheme"
          />
          <SettingsAccessTokensPage v-else-if="currentSection === 'access-tokens'" />
          <SettingsSearchPage v-else-if="currentSection === 'search'" />
          <SettingsRuntimePage v-else-if="currentSection === 'runtime'" />
          <SettingsDoclingPage v-else-if="currentSection === 'docling'" />
          <SettingsAdminUsersPage v-else />
        </div>

        <div class="settings-save-bar">
          <Button
            :class="settingsFloatingSaveButtonClass"
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
