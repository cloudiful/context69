<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";

import AppFormField from "./AppFormField.vue";
import AppTextField from "./AppTextField.vue";
import AppToggleGroup from "./AppToggleGroup.vue";
import { useErrorToast } from "../composables/use-error-toast";
import { apiClient } from "../services/api";

const props = defineProps<{ groupPath: string; canManage: boolean }>();
const { t } = useI18n();
const toast = useToast();
const showErrorToast = useErrorToast();
const enabled = ref(false);
const targets = ref("");
const sourceLocale = ref("");
const glossary = ref("");
const stats = ref({ queued: 0, running: 0, succeeded: 0, failed: 0 });
const loading = ref(false);
const saving = ref(false);

async function load() {
  loading.value = true;
  try {
    const value = await apiClient.getGroupTranslationSettings(props.groupPath);
    enabled.value = value.enabled;
    targets.value = value.default_target_locales.join(", ");
    sourceLocale.value = value.source_locale ?? "";
    glossary.value = value.glossary.map((entry) => `${entry.source} = ${entry.target}`).join("\n");
    stats.value = {
      queued: value.queued_count,
      running: value.running_count,
      succeeded: value.succeeded_count,
      failed: value.failed_count,
    };
  } catch (error) {
    showErrorToast(error, t("groups.translation.loadFailed"));
  } finally {
    loading.value = false;
  }
}

function parseGlossary() {
  return glossary.value.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
    const separator = line.indexOf("=");
    if (separator < 1 || !line.slice(separator + 1).trim()) throw new Error(t("groups.translation.invalidGlossary"));
    return { source: line.slice(0, separator).trim(), target: line.slice(separator + 1).trim() };
  });
}

async function save() {
  saving.value = true;
  try {
    await apiClient.updateGroupTranslationSettings(props.groupPath, {
      enabled: enabled.value,
      default_target_locales: targets.value.split(",").map((value) => value.trim()).filter(Boolean),
      source_locale: sourceLocale.value.trim() || undefined,
      glossary: parseGlossary(),
    });
    toast.add({ color: "success", title: t("settings.saveSuccess"), duration: 2500 });
    await load();
  } catch (error) {
    showErrorToast(error, t("settings.saveFailed"));
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <UCard>
    <div class="flex items-center justify-between gap-2">
      <strong>{{ t("groups.translation.title") }}</strong>
      <span class="text-xs text-muted-color">{{ t("groups.translation.stats", stats) }}</span>
    </div>
    <AppToggleGroup
      :model-value="{ enabled }"
      :items="[{ key: 'enabled', inputId: 'group-translation-enabled', label: t('groups.translation.enabled') }]"
      :disabled="loading || !canManage"
      @update:model-value="enabled = !!$event.enabled"
    />
    <AppTextField v-model="targets" input-id="group-translation-targets" :label="t('groups.translation.targets')" :disabled="!canManage" placeholder="zh-CN, ja-JP" />
    <AppTextField v-model="sourceLocale" input-id="group-translation-source" :label="t('groups.translation.source')" :disabled="!canManage" placeholder="en-US" />
    <AppFormField input-id="group-translation-glossary" :label="t('groups.translation.glossary')">
      <UTextarea id="group-translation-glossary" v-model="glossary" rows="4" :disabled="!canManage" placeholder="stock = 股票" />
    </AppFormField>
    <div v-if="canManage" class="flex justify-end">
      <UButton type="button" :disabled="loading || saving" :aria-busy="saving" @click="save">
        <UIcon name="i-lucide-loader-circle" v-if="saving" class="h-4 w-4" />
        <span>{{ t("common.save") }}</span>
      </UButton>
    </div>
  </UCard>
</template>
