<script setup lang="ts">
import { computed, ref, watch } from "vue";

import AppMonacoEditor from "./AppMonacoEditor.vue";

export interface SourceConfigDraft {
  source_id?: string | null;
  source_key: string;
  display_name?: string | null;
  description?: string | null;
  example_queries: string[];
  connection: string;
  database_url?: string | null;
  sync_strategy: "cursor" | "full_scan";
  connector_type: "postgres_sql";
  base_query: string;
  batch_size: number;
  visibility?: "public" | "private" | null;
}

const props = defineProps<{
  modelValue: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [string];
}>();

const mode = ref<"form" | "json">("form");
const parseError = ref<string | null>(null);

function parseDraft(value: string): SourceConfigDraft | null {
  try {
    const parsed = JSON.parse(value) as Partial<SourceConfigDraft>;
    if (typeof parsed !== "object" || parsed === null) {
      return null;
    }
    return {
      source_id: parsed.source_id ?? null,
      source_key: typeof parsed.source_key === "string" ? parsed.source_key : "",
      display_name: typeof parsed.display_name === "string" ? parsed.display_name : null,
      description: typeof parsed.description === "string" ? parsed.description : null,
      example_queries: Array.isArray(parsed.example_queries)
        ? parsed.example_queries.filter((item): item is string => typeof item === "string")
        : [],
      connection: typeof parsed.connection === "string" ? parsed.connection : "",
      database_url: typeof parsed.database_url === "string" ? parsed.database_url : null,
      sync_strategy: parsed.sync_strategy === "full_scan" ? "full_scan" : "cursor",
      connector_type: "postgres_sql",
      base_query: typeof parsed.base_query === "string" ? parsed.base_query : "",
      batch_size:
        typeof parsed.batch_size === "number" && Number.isFinite(parsed.batch_size)
          ? parsed.batch_size
          : 200,
      visibility:
        parsed.visibility === "public" || parsed.visibility === "private"
          ? parsed.visibility
          : null,
    };
  } catch {
    return null;
  }
}

function serializeDraft(draft: SourceConfigDraft): string {
  const payload: Record<string, unknown> = {
    source_key: draft.source_key,
    display_name: draft.display_name || undefined,
    description: draft.description || undefined,
    example_queries: draft.example_queries,
    connection: draft.connection,
    sync_strategy: draft.sync_strategy,
    connector_type: draft.connector_type,
    base_query: draft.base_query,
    batch_size: draft.batch_size,
  };
  if (draft.source_id) {
    payload.source_id = draft.source_id;
  }
  if (draft.database_url) {
    payload.database_url = draft.database_url;
  }
  if (draft.visibility) {
    payload.visibility = draft.visibility;
  }
  return JSON.stringify(payload, null, 2);
}

const draft = ref<SourceConfigDraft | null>(null);

watch(
  () => props.modelValue,
  (modelValue) => {
    const parsed = parseDraft(modelValue);
    parseError.value = parsed ? null : "Invalid JSON configuration";
    if (parsed) {
      draft.value = parsed;
    }
  },
  { immediate: true },
);

function updateDraft(patch: Partial<SourceConfigDraft>) {
  if (!draft.value) {
    return;
  }
  draft.value = { ...draft.value, ...patch };
  emit("update:modelValue", serializeDraft(draft.value));
}

const syncStrategyOptions = [
  { label: "cursor", value: "cursor" },
  { label: "full_scan", value: "full_scan" },
] as const;

const connectorTypeOptions = [{ label: "postgres_sql", value: "postgres_sql" }] as const;

const visibilityOptions = [
  { label: "public", value: "public" },
  { label: "private", value: "private" },
] as const;

const exampleQueriesText = computed({
  get: () => draft.value?.example_queries.join("\n") ?? "",
  set: (value: string) =>
    updateDraft({
      example_queries: value
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean),
    }),
});
</script>

<template>
  <div class="grid gap-3">
    <div class="flex items-center justify-between gap-2">
      <UTabs
        v-model="mode"
        class="w-fit"
        :items="[
          { label: $t('library.sourceConfigForm'), slot: 'form' },
          { label: $t('library.sourceConfigJson'), slot: 'json' },
        ]"
      />
      <p v-if="parseError" class="text-xs text-(--ui-error)">
        {{ parseError }}
      </p>
    </div>

    <template v-if="mode === 'form'">
      <div v-if="!draft" class="text-sm text-(--ui-text-muted)">
        {{ $t("library.sourceConfigInvalid") }}
      </div>
      <div v-else class="grid gap-4 sm:grid-cols-2">
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceKey") }}</span>
          <UInput v-model="draft.source_key" :disabled="disabled || !!draft.source_id" />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.displayName") }}</span>
          <UInput
            :model-value="draft.display_name ?? ''"
            :disabled="disabled"
            @update:model-value="updateDraft({ display_name: String($event) || null })"
          />
        </label>
        <label class="grid gap-1.5 sm:col-span-2">
          <span class="text-sm font-medium">{{ $t("library.description") }}</span>
          <UInput
            :model-value="draft.description ?? ''"
            :disabled="disabled"
            @update:model-value="updateDraft({ description: String($event) || null })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceConnection") }}</span>
          <UInput
            :model-value="draft.connection"
            :disabled="disabled"
            placeholder="gov-info"
            @update:model-value="updateDraft({ connection: String($event) })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceDatabaseUrl") }}</span>
          <UInput
            :model-value="draft.database_url ?? ''"
            :disabled="disabled"
            type="password"
            autocomplete="off"
            @update:model-value="updateDraft({ database_url: String($event) || null })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceSyncStrategy") }}</span>
          <USelect
            :model-value="draft.sync_strategy"
            :disabled="disabled"
            :options="syncStrategyOptions"
            value-key="value"
            option-attribute="label"
            @update:model-value="updateDraft({ sync_strategy: $event })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceConnectorType") }}</span>
          <USelect
            :model-value="draft.connector_type"
            :disabled="disabled"
            :options="connectorTypeOptions"
            value-key="value"
            option-attribute="label"
            @update:model-value="updateDraft({ connector_type: $event })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceVisibility") }}</span>
          <USelect
            :model-value="draft.visibility ?? 'private'"
            :disabled="disabled"
            :options="visibilityOptions"
            value-key="value"
            option-attribute="label"
            @update:model-value="updateDraft({ visibility: $event })"
          />
        </label>
        <label class="grid gap-1.5">
          <span class="text-sm font-medium">{{ $t("library.sourceBatchSize") }}</span>
          <UInput
            :model-value="String(draft.batch_size)"
            :disabled="disabled"
            type="number"
            min="1"
            @update:model-value="
              updateDraft({ batch_size: Math.max(1, Math.trunc(Number($event) || 1)) })
            "
          />
        </label>
        <label class="grid gap-1.5 sm:col-span-2">
          <span class="text-sm font-medium">{{ $t("library.sourceBaseQuery") }}</span>
          <UTextarea
            :model-value="draft.base_query"
            :disabled="disabled"
            rows="4"
            placeholder="SELECT id, title, body_text FROM documents"
            @update:model-value="updateDraft({ base_query: String($event) })"
          />
        </label>
        <label class="grid gap-1.5 sm:col-span-2">
          <span class="text-sm font-medium">{{ $t("library.sourceExampleQueries") }}</span>
          <UTextarea
            :model-value="exampleQueriesText"
            :disabled="disabled"
            rows="3"
            :placeholder="$t('library.sourceExampleQueriesPlaceholder')"
            @update:model-value="exampleQueriesText = String($event)"
          />
        </label>
      </div>
    </template>

    <template v-else>
      <div class="min-h-[26rem] overflow-hidden rounded-xl border border-surface">
        <AppMonacoEditor
          :model-value="modelValue"
          language="json"
          :disabled="disabled"
          @update:model-value="emit('update:modelValue', $event)"
        />
      </div>
    </template>
  </div>
</template>
