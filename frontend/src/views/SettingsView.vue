<script setup lang="ts">
import { computed, provide, unref } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import { authSessionState } from "../services/auth/session";
import { settingsPageStateKey } from "../composables/settings-page-context";
import { useUiPreferences } from "../composables/use-ui-preferences";
import { useSettingsPage } from "../composables/use-settings-page";
import { normalizeAppLocale, persistLocale, type AppLocale } from "../i18n/locale";
import { resolveSettingsSectionNav, type SettingsSectionKey } from "../settings/navigation";
import SettingsAccessTokensPage from "./settings/SettingsAccessTokensPage.vue";
import SettingsAdminUsersPage from "./settings/SettingsAdminUsersPage.vue";
import SettingsAppearancePage from "./settings/SettingsAppearancePage.vue";
import SettingsDoclingPage from "./settings/SettingsDoclingPage.vue";
import SettingsRuntimePage from "./settings/SettingsRuntimePage.vue";
import SettingsSearchPage from "./settings/SettingsSearchPage.vue";
import SettingsTranslationPage from "./settings/SettingsTranslationPage.vue";

const { t, locale } = useI18n({ useScope: "global" });
const route = useRoute();
const router = useRouter();
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
    case "settings-translation":
      return "translation";
    case "settings-appearance":
    default:
      return "appearance";
  }
});

const hasChanges = computed(() => unref(state.hasChanges));
const loading = computed(() => unref(state.loading));
const saveMessage = computed(() => unref(state.saveMessage));
const saving = computed(() => unref(state.saving));
const currentLocale = computed<AppLocale>(() => normalizeAppLocale(String(locale.value)) ?? "en");

const sectionItems = computed(() =>
  resolveSettingsSectionNav(t, authSessionState.user?.is_admin === true).map((section) => ({
    label: section.label,
    to: section.to,
  })),
);

function switchSection(to: string) {
  void router.push(to);
}

onBeforeRouteLeave(() => {
  if (!unref(state.hasChanges)) {
    return true;
  }
  const confirmed = window.confirm(t("settings.unsavedChanges"));
  return confirmed;
});

function switchLocale(nextLocale: AppLocale) {
  if (currentLocale.value === nextLocale) {
    return;
  }

  locale.value = nextLocale;
  persistLocale(nextLocale);
}
</script>

<template>
  <section class="grid h-full min-h-0 min-w-0 grid-rows-[auto_minmax(0,1fr)] gap-3 overflow-y-auto" data-testid="settings-page-scroll">
    <UTabs
      :model-value="currentSection"
      class="md:hidden"
      orientation="horizontal"
      :items="sectionItems"
      @update:model-value="switchSection(String($event))"
    />

    <div class="flex flex-wrap justify-end gap-1.5">
      <UBadge v-if="hasChanges" color="neutral" variant="subtle" :label="t('settings.status.pending')" />
      <UBadge v-if="saveMessage" color="success" variant="subtle" :label="saveMessage" />
    </div>

    <AsyncStateBlock
      :loading="loading"
      :loading-title="t('settings.loadingTitle')"
      :loading-message="t('settings.loadingMessage')"
    >
      <form class="grid min-w-0 gap-2 pb-24" @submit.prevent="state.saveSettings">
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
          <SettingsTranslationPage v-else-if="currentSection === 'translation'" />
          <SettingsAdminUsersPage v-else />
        </div>

        <div class="fixed right-4 bottom-20 z-40 flex justify-end md:right-6 md:bottom-6">
          <UButton
            class="min-w-28"
            data-testid="settings-save"
            type="submit"
            :disabled="saving || !hasChanges"
          >
            {{ saving ? t("common.loading") : t("common.save") }}
          </UButton>
        </div>
      </form>
    </AsyncStateBlock>
  </section>
</template>
