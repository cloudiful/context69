<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Form, FormField } from "@primevue/forms";
import { zodResolver } from "@primevue/forms/resolvers/zod";
import Button from "primevue/button";
import Fluid from "primevue/fluid";
import InputNumber from "primevue/inputnumber";
import InputText from "primevue/inputtext";
import Password from "primevue/password";
import Select from "primevue/select";
import Message from "primevue/message";
import Textarea from "primevue/textarea";
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

const resolver = computed(() =>
  zodResolver(
    z.object({
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
    }),
  ),
);

const formKey = computed(() => `${props.source?.source_key ?? "new"}:${props.connections.map((connection) => connection.name).join("|")}`);

function normalizeSubmitValues(values: Record<string, any>): SourceConfigInput {
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

function handleSubmit(event: { valid: boolean; values: Record<string, any> }) {
  if (!event.valid) {
    return;
  }

  emit("save", normalizeSubmitValues(event.values));
}
</script>

<template>
  <Fluid>
    <Form
      :key="formKey"
      class="grid min-h-0 gap-4 [grid-template-rows:minmax(0,1fr)_auto]"
      :initial-values="initialValues"
      :resolver="resolver"
      @submit="handleSubmit"
    >
      <div class="grid min-h-0 gap-4 [grid-template-columns:minmax(19rem,22rem)_minmax(0,1fr)] max-md:grid-cols-1">
        <section class="min-h-0 min-w-0">
          <div class="rounded-[1.1rem] border border-(--p-content-border-color)/80 bg-(--p-content-hover-background)/36 p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]">
            <div class="mb-4 grid gap-1">
              <p class="text-base font-semibold text-(--p-text-color)">{{ t("sources.form.sourceSectionTitle") }}</p>
              <p class="text-xs leading-6 text-(--p-text-muted-color)">{{ t("sources.form.sourceSectionDescription") }}</p>
            </div>

            <div class="grid gap-4">
              <FormField v-slot="$field" name="source_key" :initial-value="initialValues.source_key">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.sourceKey") }}</span>
                  <InputText
                    id="source-key"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :disabled="!!props.source"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                    {{ $field.error?.message }}
                  </Message>
                </label>
              </FormField>

              <p class="text-xs leading-6 text-(--p-text-muted-color)">{{ t("sources.form.lockedHint") }}</p>

              <FormField v-slot="$field" name="display_name" :initial-value="initialValues.display_name">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.displayName") }}</span>
                  <InputText
                    id="source-display-name"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :placeholder="t('sources.form.displayNamePlaceholder')"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                </label>
              </FormField>

              <FormField v-slot="$field" name="description" :initial-value="initialValues.description">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.description") }}</span>
                  <Textarea
                    id="source-description"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    rows="4"
                    :placeholder="t('sources.form.descriptionPlaceholder')"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                </label>
              </FormField>

              <FormField v-slot="$field" name="example_queries_text" :initial-value="initialValues.example_queries_text">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.exampleQueries") }}</span>
                  <Textarea
                    id="source-example-queries"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    rows="5"
                    :placeholder="t('sources.form.exampleQueriesPlaceholder')"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <p class="text-xs leading-6 text-(--p-text-muted-color)">{{ t("sources.form.exampleQueriesHint") }}</p>
                  <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                    {{ $field.error?.message }}
                  </Message>
                </label>
              </FormField>

              <FormField v-slot="$field" name="connection" :initial-value="initialValues.connection">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.connection") }}</span>
                  <Select
                    id="source-connection"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :options="props.connections"
                    option-label="name"
                    option-value="name"
                    :placeholder="t('sources.form.connectionPlaceholder')"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                    {{ $field.error?.message }}
                  </Message>
                </label>
              </FormField>

              <FormField v-slot="$field" name="database_url" :initial-value="initialValues.database_url">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.databaseUrl") }}</span>
                  <Password
                    id="source-database-url"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :feedback="false"
                    toggle-mask
                    :placeholder="t('sources.form.databaseUrlPlaceholder')"
                    autocomplete="new-password"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <p class="text-xs leading-6 text-(--p-text-muted-color)">{{ t("sources.form.databaseUrlHint") }}</p>
                </label>
              </FormField>

              <FormField v-slot="$field" name="sync_strategy" :initial-value="initialValues.sync_strategy">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.strategy") }}</span>
                  <Select
                    id="source-strategy"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :options="strategyOptions"
                    option-label="label"
                    option-value="value"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                </label>
              </FormField>

              <FormField v-slot="$field" name="connector_type" :initial-value="initialValues.connector_type">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.connector") }}</span>
                  <InputText
                    id="source-connector"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    readonly
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                    {{ $field.error?.message }}
                  </Message>
                </label>
              </FormField>

              <FormField v-slot="$field" name="batch_size" :initial-value="initialValues.batch_size">
                <label class="grid min-w-0 content-start self-start gap-3">
                  <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.batchSize") }}</span>
                  <InputNumber
                    id="source-batch-size"
                    v-bind="$field.props"
                    :model-value="$field.value"
                    fluid
                    :min="1"
                    :use-grouping="false"
                    @update:model-value="$field.props.onInput({ value: $event })"
                  />
                  <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                    {{ $field.error?.message }}
                  </Message>
                </label>
              </FormField>
            </div>
          </div>
        </section>

        <section class="min-h-0 min-w-0">
          <div class="grid h-full min-h-0 gap-3 p-3">
            <div class="grid gap-1">
              <div>
                <p class="text-base font-semibold text-(--p-text-color)">{{ t("sources.form.queryTitle") }}</p>
                <p class="text-xs leading-6 text-(--p-text-muted-color)">{{ t("sources.form.queryDescription") }}</p>
              </div>
            </div>

            <FormField v-slot="$field" name="base_query" :initial-value="initialValues.base_query">
              <label class="grid min-w-0 content-start self-start gap-3">
                <span class="text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("sources.form.baseQuery") }}</span>
                <AppMonacoEditor
                  input-id="source-base-query"
                  :model-value="$field.value"
                  language="sql"
                  :placeholder="t('sources.form.queryPlaceholder')"
                  @update:model-value="$field.props.onInput({ value: $event })"
                />
                <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                  {{ $field.error?.message }}
                </Message>
              </label>
            </FormField>
          </div>
        </section>
      </div>

      <div class="flex flex-wrap items-center gap-3 border-t border-(--p-content-border-color)/70 pt-4">
        <Button class="min-w-32" type="submit" :disabled="busy">
          {{ props.source ? t("sources.form.save") : t("sources.form.create") }}
        </Button>
        <Button class="min-w-24" type="button" severity="secondary" variant="outlined" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </Button>
      </div>
    </Form>
  </Fluid>
</template>
