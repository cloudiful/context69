<script setup lang="ts">
import type { AppLocale } from "../../i18n/locale";
import type { AppTheme } from "../../types/ui";

import SettingsAppearanceSection from "../../components/settings-sections/SettingsAppearanceSection.vue";

defineProps<{
  locale: AppLocale;
  theme: AppTheme;
  timeZone: string;
}>();

defineEmits<{
  "update:locale": [value: AppLocale];
  "update:theme": [value: AppTheme];
  "update:timeZone": [value: string];
}>();
</script>

<template>
  <SettingsAppearanceSection
    :locale="locale"
    :theme="theme"
    :time-zone="timeZone"
    @update:locale="$emit('update:locale', $event)"
    @update:theme="$emit('update:theme', $event)"
    @update:time-zone="$emit('update:timeZone', $event)"
  />
</template>
