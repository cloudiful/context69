<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import { normalizeAppLocale, type AppLocale } from "../../i18n/locale";
import type { AppTheme } from "../../types/ui";

const props = defineProps<{
  locale: AppLocale;
  theme: AppTheme;
}>();

const emit = defineEmits<{
  "update:locale": [value: AppLocale];
  "update:theme": [value: AppTheme];
}>();

const { t } = useI18n();

const localeOptions = computed<Array<{ value: AppLocale; label: string }>>(() => [
  { value: "en", label: t("language.en") },
  { value: "zh-CN", label: t("language.zhCN") },
]);

const themeOptions = computed<Array<{ value: AppTheme; label: string }>>(() => [
  { value: "dark", label: t("theme.dark") },
  { value: "light", label: t("theme.light") },
]);

const selectedLocale = computed<AppLocale>(() => normalizeAppLocale(props.locale) ?? "en");
const localeSelectKey = computed(() => `${selectedLocale.value}:${localeOptions.value.map((option) => option.label).join("|")}`);

function updateLocale(value: unknown) {
  if (value === "en" || value === "zh-CN") {
    emit("update:locale", value);
  }
}

function updateTheme(value: unknown) {
  if (value === "dark" || value === "light") {
    emit("update:theme", value);
  }
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.appearance.title')">
    <AppSettingsBlock id="settings-appearance">
      <div class="grid gap-4 lg:grid-cols-2 lg:items-start xl:grid-cols-[repeat(2,minmax(18rem,24rem))] xl:justify-start">
        <AppSelectField
          :key="localeSelectKey"
          float-label
          input-id="settings-locale-select"
          :model-value="selectedLocale"
          :label="t('language.label')"
          :options="localeOptions"
          test-id="settings-locale-select"
          @update:model-value="updateLocale"
        />

        <AppSelectField
          float-label
          input-id="settings-theme-select"
          :model-value="theme"
          :label="t('theme.label')"
          :options="themeOptions"
          test-id="settings-theme-select"
          @update:model-value="updateTheme"
        />
      </div>
    </AppSettingsBlock>
  </AppSettingsSection>
</template>
