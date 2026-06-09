<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import SelectButton from "primevue/selectbutton";

import { persistLocale, type AppLocale } from "../i18n/locale";
import { useUiPreferences } from "../composables/use-ui-preferences";
import type { AppTheme } from "../types/ui";

const { t, locale } = useI18n({ useScope: "global" });
const preferences = useUiPreferences();

const localeOptions = computed<Array<{ value: AppLocale; label: string }>>(() => [
  { value: "en", label: t("language.en") },
  { value: "zh-CN", label: t("language.zhCN") },
]);

const themeOptions = computed<Array<{ value: AppTheme; label: string }>>(() => [
  { value: "dark", label: t("theme.dark") },
  { value: "light", label: t("theme.light") },
]);

function isAppLocale(value: unknown): value is AppLocale {
  return value === "en" || value === "zh-CN";
}

function isAppTheme(value: unknown): value is AppTheme {
  return value === "dark" || value === "light";
}

function switchLocale(nextLocale: unknown) {
  if (!isAppLocale(nextLocale)) {
    return;
  }

  if (locale.value === nextLocale) {
    return;
  }

  locale.value = nextLocale;
  persistLocale(nextLocale);
}

function switchTheme(nextTheme: unknown) {
  if (!isAppTheme(nextTheme)) {
    return;
  }

  if (preferences.state.theme === nextTheme) {
    return;
  }

  preferences.setTheme(nextTheme);
}
</script>

<template>
  <div class="footer-tools">
    <div class="footer-tools-section">
      <SelectButton
        class="footer-tools-select"
        data-testid="locale-select"
        :model-value="locale"
        :options="localeOptions"
        :allow-empty="false"
        option-label="label"
        option-value="value"
        @update:model-value="switchLocale"
      />
    </div>

    <div class="footer-tools-section">
      <SelectButton
        class="footer-tools-select"
        data-testid="theme-select"
        :model-value="preferences.state.theme"
        :options="themeOptions"
        :allow-empty="false"
        option-label="label"
        option-value="value"
        @update:model-value="switchTheme"
      />
    </div>
  </div>
</template>
