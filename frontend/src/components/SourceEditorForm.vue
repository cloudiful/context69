<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import type { FormSubmitEvent } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import { z } from "zod";

import AppMonacoEditor from "./AppMonacoEditor.vue";
import type { SourceConfigInput, SourceConnectionResponse, SourceStatus } from "../services/api";

const props = defineProps<{
  source: SourceStatus | null;
  connections: SourceConnectionResponse[];
  busy: boolean;
}>();

const emit = defineEmits<{
  save: [SourceConfigInput];
  cancel: [];
}>();

const { t } = useI18n();

const strategyOptions = [
  { value: "cursor", label: "cursor" },
  { value: "full_scan", label: "full_scan" },
];

const MAX_EXAMPLE_QUERIES = 6;
const MAX_EXAMPLE_QUERY_LEN = 120;

function parseExampleQueriesText(value: unknown): string[] {
  return String(value ?? "")
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean)
    .filter((item, index, items) => items.indexOf(item) === index);
}

const initialValues = computed(() => ({
  source_key: props.source?.source_key ?? "",
  display_name: props.source && props.source.display_name !== props.source.source_key
    ? props.source.display_name
    : "",
  description: props.source?.description ?? "",
  example_queries_text: (props.source?.example_queries ?? []).join("\n"),
  connection: props.source?.connection ?? props.connections[0]?.name ?? "",
  database_url: "",
  sync_strategy: props.source?.sync_strategy ?? "cursor",
  connector_type: props.source?.connector_type ?? "postgres_sql",
  batch_size: props.source?.batch_size ?? 200,
  base_query: props.source?.base_query ?? "",
}));

const schema = computed(() => z.object({
      source_key: z.string().trim().min(1, { message: t("sources.form.validation.sourceKeyRequired") }),
      display_name: z.string().optional(),
      description: z.string().optional(),
      example_queries_text: z.string().superRefine((value, ctx) => {
        const queries = parseExampleQueriesText(value);
        if (queries.length > MAX_EXAMPLE_QUERIES) {
          ctx.addIssue({
            code: "custom",
            message: t("sources.form.validation.exampleQueriesMaximum", { count: MAX_EXAMPLE_QUERIES }),
          });
        }
        if (queries.some((query) => query.length > MAX_EXAMPLE_QUERY_LEN)) {
          ctx.addIssue({
            code: "custom",
            message: t("sources.form.validation.exampleQueryTooLong", { count: MAX_EXAMPLE_QUERY_LEN }),
          });
        }
      }),
      connection: z.string().trim().min(1, { message: t("sources.form.validation.connectionRequired") }),
      database_url: z.string().optional(),
      sync_strategy: z.enum(["cursor", "full_scan"]),
      connector_type: z.string().trim().min(1, { message: t("sources.form.validation.connectorRequired") }),
      batch_size: z.coerce.number().int().min(1, { message: t("sources.form.validation.batchSizeMinimum") }),
      base_query: z.string().trim().min(1, { message: t("sources.form.validation.baseQueryRequired") }),
}));

const state = reactive(initialValues.value);
watch(initialValues, (values) => Object.assign(state, values), { immediate: true });

const formKey = computed(() => `${props.source?.source_key ?? "new"}:${props.connections.map((connection) => connection.name).join("|")}`);

function normalizeSubmitValues(values: Record<string, unknown>): SourceConfigInput {
  return {
    source_key: String(values.source_key ?? "").trim(),
    display_name: String(values.display_name ?? "").trim() || undefined,
    description: String(values.description ?? "").trim() || undefined,
    example_queries: parseExampleQueriesText(values.example_queries_text),
    connection: String(values.connection ?? "").trim(),
    database_url: String(values.database_url ?? "").trim() || undefined,
    sync_strategy: String(values.sync_strategy ?? "cursor"),
    connector_type: String(values.connector_type ?? "").trim(),
    batch_size: Number(values.batch_size ?? 0),
    base_query: String(values.base_query ?? "").trim(),
  };
}

function handleSubmit(event: FormSubmitEvent<Record<string, unknown>>) {
  emit("save", normalizeSubmitValues(event.data));
}
</script>

<template>
  <div>
    <UForm
      :key="formKey"
      class="grid min-h-0 gap-4 [grid-template-rows:minmax(0,1fr)_auto]"
      :state="state"
      :schema="schema"
      @submit="handleSubmit"
    >
      <div class="grid min-h-0 gap-4 [grid-template-columns:minmax(19rem,22rem)_minmax(0,1fr)] max-md:grid-cols-1">
        <section class="min-h-0 min-w-0">
          <div class="rounded-[1.1rem] border border-surface bg-emphasis p-3">
            <div class="mb-4 grid gap-1">
              <p class="text-base font-semibold text-color">{{ t("sources.form.sourceSectionTitle") }}</p>
              <p class="text-xs leading-6 text-muted-color">{{ t("sources.form.sourceSectionDescription") }}</p>
            </div>

            <div class="grid gap-4">
              <UFormField name="source_key">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.sourceKey") }}</span>
                  <UInput
                    id="source-key"

                    v-model="state.source_key"
                    :disabled="!!props.source"
                  />
                </label>
              </UFormField>

              <p class="text-xs leading-6 text-muted-color">{{ t("sources.form.lockedHint") }}</p>

              <UFormField name="display_name">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.displayName") }}</span>
                  <UInput
                    id="source-display-name"

                    v-model="state.display_name"
                    :placeholder="t('sources.form.displayNamePlaceholder')"
                  />
                </label>
              </UFormField>

              <UFormField name="description">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.description") }}</span>
                  <UTextarea
                    id="source-description"

                    v-model="state.description"
                    rows="4"
                    :placeholder="t('sources.form.descriptionPlaceholder')"
                  />
                </label>
              </UFormField>

              <UFormField name="example_queries_text">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.exampleQueries") }}</span>
                  <UTextarea
                    id="source-example-queries"

                    v-model="state.example_queries_text"
                    rows="5"
                    :placeholder="t('sources.form.exampleQueriesPlaceholder')"
                  />
                  <p class="text-xs leading-6 text-muted-color">{{ t("sources.form.exampleQueriesHint") }}</p>
                </label>
              </UFormField>

              <UFormField name="connection">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.connection") }}</span>
                  <USelect
                    id="source-connection"

                    v-model="state.connection"
                    :items="props.connections"
                    label-key="name"
                    value-key="name"
                    :placeholder="t('sources.form.connectionPlaceholder')"
                  />
                </label>
              </UFormField>

              <UFormField name="database_url">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.databaseUrl") }}</span>
                  <UInput type="password"
                    id="source-database-url"

                    v-model="state.database_url"
                    :placeholder="t('sources.form.databaseUrlPlaceholder')"
                    autocomplete="new-password"
                  />
                  <p class="text-xs leading-6 text-muted-color">{{ t("sources.form.databaseUrlHint") }}</p>
                </label>
              </UFormField>

              <UFormField name="sync_strategy">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.strategy") }}</span>
                  <USelect
                    id="source-strategy"

                    v-model="state.sync_strategy"
                    :items="strategyOptions"
                    label-key="label"
                    value-key="value"
                  />
                </label>
              </UFormField>

              <UFormField name="connector_type">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.connector") }}</span>
                  <UInput
                    id="source-connector"

                    v-model="state.connector_type"
                    readonly
                  />
                </label>
              </UFormField>

              <UFormField name="batch_size">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.batchSize") }}</span>
                  <UInputNumber
                    id="source-batch-size"

                    v-model="state.batch_size"
                    :min="1"
                  />
                </label>
              </UFormField>
            </div>
          </div>
        </section>

        <section class="min-h-0 min-w-0">
          <div class="grid h-full min-h-0 gap-3 p-3">
            <div class="grid gap-1">
              <div>
                <p class="text-base font-semibold text-color">{{ t("sources.form.queryTitle") }}</p>
                <p class="text-xs leading-6 text-muted-color">{{ t("sources.form.queryDescription") }}</p>
              </div>
            </div>

            <UFormField name="base_query">
              <label class="grid min-w-0 content-start self-start gap-3">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("sources.form.baseQuery") }}</span>
                <AppMonacoEditor
                  input-id="source-base-query"
                  v-model="state.base_query"
                  language="sql"
                  :placeholder="t('sources.form.queryPlaceholder')"
                />
              </label>
            </UFormField>
          </div>
        </section>
      </div>

      <div class="flex flex-wrap items-center gap-3 border-t border-surface pt-4">
        <UButton class="min-w-32" type="submit" :disabled="busy">
          {{ props.source ? t("sources.form.save") : t("sources.form.create") }}
        </UButton>
        <UButton class="min-w-24" type="button" color="neutral" variant="outline" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </UButton>
      </div>
    </UForm>
  </div>
</template>
