<script setup lang="ts">
import { computed, ref, unref } from "vue";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import Tag from "primevue/tag";
import ToggleSwitch from "primevue/toggleswitch";
import { useI18n } from "vue-i18n";

import AppNumberField from "../../components/AppNumberField.vue";
import AppSelectField from "../../components/AppSelectField.vue";
import AppSettingsSection from "../../components/AppSettingsSection.vue";
import AppTextField from "../../components/AppTextField.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";
import type { TranslationProviderInput } from "../../services/api";

type ProviderDraft = Omit<TranslationProviderInput, "enabled"> & {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
};

const { t } = useI18n();
const state = useSettingsPageContext();
const providers = computed(() => unref(state.translationProviders));
const dialogVisible = ref(false);
const editingIndex = ref(-1);
const editing = ref<ProviderDraft | null>(null);

const providerLabels = { deepl: "DeepL", llm: "LLM", libretranslate: "LibreTranslate" } as const;
const llmKinds = [
  { label: "OpenAI Responses", value: "openai_responses" },
  { label: "OpenAI Chat Completions", value: "openai_chat_completions" },
  { label: "Anthropic Messages", value: "anthropic_messages" },
];
const deeplPlans = [{ label: "Developer", value: "free" }, { label: "Growth / Enterprise", value: "pro" }];

function openEdit(provider: ProviderDraft, index: number) {
  editingIndex.value = index;
  editing.value = { ...provider };
  dialogVisible.value = true;
}

function applyEdit() {
  if (!editing.value || editingIndex.value < 0) return;
  providers.value.splice(editingIndex.value, 1, editing.value);
  dialogVisible.value = false;
}

function move(index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= providers.value.length) return;
  const [provider] = providers.value.splice(index, 1);
  providers.value.splice(target, 0, provider);
  providers.value.forEach((item, priority) => { item.priority = priority; });
}

function quotaLabel(provider: ProviderDraft) {
  if (provider.provider === "libretranslate") return t("settings.translation.unlimited");
  return provider.monthly_character_limit?.toLocaleString() ?? t("settings.translation.unlimited");
}

function usageLabel(provider: ProviderDraft) {
  return provider.provider === "deepl"
    ? t("settings.translation.lifetimeUsage", { count: provider.current_month_characters })
    : t("settings.translation.currentUsage", { count: provider.current_month_characters });
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.translation.title')">
    <DataTable class="min-w-0 max-w-full" :value="providers" data-key="provider" size="small" scrollable table-class="min-w-full">
      <Column :header="t('settings.translation.provider')">
        <template #body="{ data }"><strong>{{ providerLabels[data.provider as keyof typeof providerLabels] }}</strong></template>
      </Column>
      <Column :header="t('settings.translation.enabled')" body-class="w-24">
        <template #body="{ data }">
          <ToggleSwitch :input-id="`translation-${data.provider}-enabled`" v-model="data.enabled" />
        </template>
      </Column>
      <Column :header="t('settings.translation.quota')">
        <template #body="{ data }"><span class="whitespace-nowrap">{{ quotaLabel(data) }}</span></template>
      </Column>
      <Column :header="t('settings.translation.usage')">
        <template #body="{ data }"><span class="whitespace-nowrap text-muted-color">{{ usageLabel(data) }}</span></template>
      </Column>
      <Column :header="t('settings.translation.configuration')">
        <template #body="{ data }">
          <Tag :value="data.endpoint ? t('settings.translation.configured') : t('settings.translation.notConfigured')" :severity="data.endpoint ? 'success' : 'secondary'" />
        </template>
      </Column>
      <Column :header="t('common.actions')" body-class="w-36">
        <template #body="{ data, index }">
          <div class="flex items-center gap-1">
            <Button type="button" severity="secondary" text :aria-label="t('common.edit')" @click="openEdit(data, index)"><i class="pi pi-pencil" aria-hidden="true" /></Button>
            <Button type="button" severity="secondary" text :disabled="index === 0" :aria-label="t('common.move')" @click="move(index, -1)"><i class="pi pi-arrow-up" aria-hidden="true" /></Button>
            <Button type="button" severity="secondary" text :disabled="index === providers.length - 1" :aria-label="t('common.move')" @click="move(index, 1)"><i class="pi pi-arrow-down" aria-hidden="true" /></Button>
          </div>
        </template>
      </Column>
    </DataTable>

    <Dialog v-model:visible="dialogVisible" modal :header="editing ? providerLabels[editing.provider] : ''" class="w-[38rem] max-w-[96vw]">
      <div v-if="editing" class="grid gap-3">
        <AppTextField v-model="editing.endpoint" :input-id="`translation-${editing.provider}-endpoint`" :label="t('settings.translation.endpoint')" type="url" />
        <AppTextField v-model="editing.api_key" :input-id="`translation-${editing.provider}-api-key`" :label="t('settings.translation.apiKey')" type="password" autocomplete="new-password" :placeholder="editing.has_api_key ? t('settings.translation.keyStored') : ''" />
        <AppTextField v-if="editing.provider === 'llm'" v-model="editing.model" input-id="translation-llm-model" :label="t('settings.translation.model')" />
        <AppSelectField v-if="editing.provider === 'llm'" v-model="editing.llm_api_kind" input-id="translation-llm-api-kind" :label="t('settings.translation.apiKind')" :options="llmKinds" />
        <AppSelectField v-if="editing.provider === 'deepl'" v-model="editing.deepl_plan" input-id="translation-deepl-plan" :label="t('settings.translation.plan')" :options="deeplPlans" />
        <AppNumberField v-if="editing.provider !== 'libretranslate'" v-model="editing.monthly_character_limit" :input-id="`translation-${editing.provider}-quota`" :label="editing.provider === 'deepl' ? t('settings.translation.lifetimeQuota') : t('settings.translation.monthlyQuota')" :min="1" :step="1000" />
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button type="button" severity="secondary" variant="outlined" @click="dialogVisible = false">{{ t("common.cancel") }}</Button>
          <Button type="button" @click="applyEdit">{{ t("common.confirm") }}</Button>
        </div>
      </template>
    </Dialog>
  </AppSettingsSection>
</template>
