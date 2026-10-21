<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import AppFormField from "../AppFormField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import { normalizeAppLocale, type AppLocale } from "../../i18n/locale";
import type { AppTheme } from "../../types/ui";
import {
  AUTOMATIC_TIME_ZONE,
  browserTimeZone,
  isValidTimeZone,
  listSupportedTimeZones,
} from "../../utils/time-zone";

const props = defineProps<{
  locale: AppLocale;
  theme: AppTheme;
  timeZone: string;
}>();

const emit = defineEmits<{
  "update:locale": [value: AppLocale];
  "update:theme": [value: AppTheme];
  "update:timeZone": [value: string];
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

const timeZoneOptions = computed<Array<{ value: string; label: string }>>(() => [
  {
    value: AUTOMATIC_TIME_ZONE,
    label: t("settings.appearance.timeZoneAutomatic", { zone: browserTimeZone }),
  },
  ...listSupportedTimeZones().map((zone) => ({ value: zone, label: zone })),
]);

const selectedTimeZone = computed(() => (
  props.timeZone === AUTOMATIC_TIME_ZONE || isValidTimeZone(props.timeZone)
    ? props.timeZone
    : AUTOMATIC_TIME_ZONE
));

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

function updateTimeZone(value: unknown) {
  if (typeof value !== "string") {
    return;
  }

  if (value === AUTOMATIC_TIME_ZONE || isValidTimeZone(value)) {
    emit("update:timeZone", value);
  }
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.appearance.title')">
    <AppSettingsBlock id="settings-appearance">
      <div class="grid max-w-sm gap-3">
        <AppSelectField
          :key="localeSelectKey"
          input-id="settings-locale-select"
          name="appearance.locale"
          :model-value="selectedLocale"
          :label="t('language.label')"
          :options="localeOptions"
          test-id="settings-locale-select"
          @update:model-value="updateLocale"
        />

        <AppSelectField
          input-id="settings-theme-select"
          name="appearance.theme"
          :model-value="theme"
          :label="t('theme.label')"
          :options="themeOptions"
          test-id="settings-theme-select"
          @update:model-value="updateTheme"
        />

        <AppFormField
          input-id="settings-time-zone-select"
          name="appearance.time_zone"
          :label="t('settings.appearance.timeZoneLabel')"
        >
          <USelectMenu
            id="settings-time-zone-select"
            name="appearance.time_zone"
            :model-value="selectedTimeZone"
            :items="timeZoneOptions"
            value-key="value"
            label-key="label"
            data-testid="settings-time-zone-select"
            @update:model-value="updateTimeZone"
          />
        </AppFormField>
      </div>
    </AppSettingsBlock>
  </AppSettingsSection>
</template>
