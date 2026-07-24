<script setup lang="ts">
import { computed, onMounted, ref, unref } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AppNumberField from "../../components/AppNumberField.vue";
import AppSelectField from "../../components/AppSelectField.vue";
import AppSettingsSection from "../../components/AppSettingsSection.vue";
import AppTextField from "../../components/AppTextField.vue";
import TablePagination from "../../components/TablePagination.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";
import type { TranslationProviderInput } from "../../services/api";
import { apiClient, type TranslationProviderPageResponse } from "../../services/api";

type ProviderDraft = Omit<TranslationProviderInput, "enabled"> & {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
};

const { t } = useI18n();
const state = useSettingsPageContext();
const providers = computed(() => unref(state.translationProviders));
const providerPage = ref<TranslationProviderPageResponse | null>(null);
const providerPageNumber = ref(1);
const providerPageSize = ref(50);
const providerLoading = ref(false);
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
const columns = computed<TableColumn<ProviderDraft>[]>(() => [
  { id: "provider", header: t("settings.translation.provider") },
  { id: "enabled", header: t("settings.translation.enabled") },
  { id: "quota", header: t("settings.translation.quota") },
  { id: "usage", header: t("settings.translation.usage") },
  { id: "configuration", header: t("settings.translation.configuration") },
  { id: "actions", header: t("common.actions") },
]);

const visibleProviders = computed(() => {
  const pageProviders = providerPage.value?.items ?? [];
  return pageProviders
    .map((provider) => providers.value.find((item) => item.provider === provider.provider))
    .filter((provider): provider is ProviderDraft => !!provider);
});

async function loadProviderPage() {
  providerLoading.value = true;
  try {
    providerPage.value = await apiClient.listTranslationProviders({
      page: providerPageNumber.value,
      pageSize: providerPageSize.value,
    });
  } finally {
    providerLoading.value = false;
  }
}

function changeProviderPage(page: number) {
  providerPageNumber.value = page;
  void loadProviderPage();
}

function changeProviderPageSize(value: number) {
  if (providerPageSize.value === value) return;
  providerPageSize.value = value;
  providerPageNumber.value = 1;
  void loadProviderPage();
}

onMounted(() => { void loadProviderPage(); });

function providerIndex(provider: ProviderDraft) {
  return providers.value.indexOf(provider);
}

function openEdit(provider: ProviderDraft) {
  const index = providerIndex(provider);
  if (index < 0) return;
  editingIndex.value = index;
  editing.value = { ...provider };
  dialogVisible.value = true;
}

function applyEdit() {
  if (!editing.value || editingIndex.value < 0) return;
  providers.value.splice(editingIndex.value, 1, editing.value);
  dialogVisible.value = false;
}

function move(provider: ProviderDraft, direction: -1 | 1) {
  const index = providerIndex(provider);
  const target = index + direction;
  if (target < 0 || target >= providers.value.length) return;
  const [movedProvider] = providers.value.splice(index, 1);
  providers.value.splice(target, 0, movedProvider);
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
    <UTable class="min-w-0 max-w-full" :data="visibleProviders" :columns="columns" :loading="providerLoading">
      <template #provider-cell="{ row }"><strong>{{ providerLabels[row.original.provider as keyof typeof providerLabels] }}</strong></template>
      <template #enabled-cell="{ row }"><USwitch :id="`translation-${row.original.provider}-enabled`" v-model="row.original.enabled" /></template>
      <template #quota-cell="{ row }"><span class="whitespace-nowrap">{{ quotaLabel(row.original) }}</span></template>
      <template #usage-cell="{ row }"><span class="whitespace-nowrap text-muted">{{ usageLabel(row.original) }}</span></template>
      <template #configuration-cell="{ row }"><UBadge :label="row.original.endpoint ? t('settings.translation.configured') : t('settings.translation.notConfigured')" :color="row.original.endpoint ? 'success' : 'neutral'" variant="subtle" /></template>
      <template #actions-cell="{ row }">
          <div class="flex items-center gap-1">
            <UButton icon="i-lucide-pencil" color="neutral" variant="ghost" :aria-label="t('common.edit')" @click="openEdit(row.original)" />
            <UButton icon="i-lucide-arrow-up" color="neutral" variant="ghost" :disabled="providerIndex(row.original) === 0" :aria-label="t('common.move')" @click="move(row.original, -1)" />
            <UButton icon="i-lucide-arrow-down" color="neutral" variant="ghost" :disabled="providerIndex(row.original) === providers.length - 1" :aria-label="t('common.move')" @click="move(row.original, 1)" />
          </div>
      </template>
    </UTable>

    <TablePagination
      :page="providerPageNumber"
      :page-size="providerPageSize"
      :total="providerPage?.total ?? 0"
      @update:page="changeProviderPage"
      @update:page-size="changeProviderPageSize"
    />

    <UModal v-model:open="dialogVisible"  :title="editing ? providerLabels[editing.provider] : ''" class="w-[38rem] max-w-[96vw]">
    <template #body>
<div v-if="editing" class="grid gap-3">
        <AppTextField v-model="editing.endpoint" :input-id="`translation-${editing.provider}-endpoint`" :label="t('settings.translation.endpoint')" type="url" />
        <AppTextField v-model="editing.api_key" :input-id="`translation-${editing.provider}-api-key`" :label="t('settings.translation.apiKey')" type="password" autocomplete="new-password" :placeholder="editing.has_api_key ? t('settings.translation.keyStored') : ''" />
        <AppTextField v-if="editing.provider === 'llm'" v-model="editing.model" input-id="translation-llm-model" :label="t('settings.translation.model')" />
        <AppSelectField v-if="editing.provider === 'llm'" v-model="editing.llm_api_kind" input-id="translation-llm-api-kind" :label="t('settings.translation.apiKind')" :options="llmKinds" />
        <AppSelectField v-if="editing.provider === 'deepl'" v-model="editing.deepl_plan" input-id="translation-deepl-plan" :label="t('settings.translation.plan')" :options="deeplPlans" />
        <AppNumberField v-if="editing.provider !== 'libretranslate'" v-model="editing.monthly_character_limit" :input-id="`translation-${editing.provider}-quota`" :label="editing.provider === 'deepl' ? t('settings.translation.lifetimeQuota') : t('settings.translation.monthlyQuota')" :min="1" :step="1000" />
      </div>
    </template>

      <template #footer>
        <div class="flex justify-end gap-2">
          <UButton type="button" color="neutral" variant="outline" @click="dialogVisible = false">{{ t("common.cancel") }}</UButton>
          <UButton type="button" @click="applyEdit">{{ t("common.confirm") }}</UButton>
        </div>
      </template>
    </UModal>
  </AppSettingsSection>
</template>
