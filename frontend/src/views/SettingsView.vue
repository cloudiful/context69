<script setup lang="ts">
import { computed, provide, unref } from "vue";
import { useRoute } from "vue-router";
import Button from "primevue/button";
import Message from "primevue/message";
import { useI18n } from "vue-i18n";

import AppMdiIcon from "../components/AppMdiIcon.vue";
import AppPanel from "../components/AppPanel.vue";
import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import { settingsPageStateKey } from "../composables/settings-page-context";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { useSettingsPage } from "../composables/use-settings-page";
import { persistLocale, type AppLocale } from "../i18n/locale";
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
const mdiMenu = "M3,6H21V8H3V6M3,11H21V13H3V11M3,16H21V18H3V16Z";
const appLocale = locale as { value: AppLocale };
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
      <form class="grid gap-2" @submit.prevent="state.saveSettings">
        <div class="settings-sections">
          <SettingsAppearancePage
            v-if="currentSection === 'appearance'"
            :locale="appLocale.value"
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
