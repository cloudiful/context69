<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import AppTableToolbar from "./AppTableToolbar.vue";
import type { SourceStatus } from "../services/api";
import {
  compactTableActionButtonClass,
  toolPrimaryButtonClass,
  toolSecondaryButtonClass,
} from "../ui/button-classes";
import { formatTimestamp } from "../utils/format";

const props = withDefaults(defineProps<{
  sources: SourceStatus[];
  syncingMap: Record<string, boolean>;
  deletingMap: Record<string, boolean>;
  canManage?: boolean;
}>(), {
  canManage: true,
});

const emit = defineEmits<{
  create: [];
  refresh: [];
  sync: [string];
  edit: [SourceStatus];
  delete: [string];
  select: [SourceStatus];
}>();

const { t } = useI18n();
const searchQuery = ref("");

const filteredSources = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  if (!query) {
    return props.sources;
  }

  return props.sources.filter((source) => sourceQueryFields(source).some((value) => value.toLowerCase().includes(query)));
});

function sourceQueryFields(source: SourceStatus) {
  const exampleQueries = source.example_queries ?? [];
  return [
    source.source_key,
    source.display_name,
    source.description ?? "",
    ...exampleQueries,
    source.connection,
    source.origin_status,
    source.origin_message ?? "",
    source.connector_type,
    source.sync_strategy,
    String(source.batch_size),
    source.last_cursor_external_id ?? "",
    source.last_success_at ?? "",
  ];
}

function originSeverity(source: SourceStatus) {
  if (source.origin_status === "connected") {
    return "success";
  }
  if (source.origin_status === "unreachable" || source.origin_status === "misconfigured") {
    return "danger";
  }
  return "secondary";
}

function originLabel(source: SourceStatus) {
  if (source.origin_status === "connected") {
    return t("sources.origin.connected");
  }
  if (source.origin_status === "unreachable") {
    return t("sources.origin.unreachable");
  }
  if (source.origin_status === "misconfigured") {
    return t("sources.origin.misconfigured");
  }
  return t("sources.origin.unknown");
}

function handleRowSelect(source: SourceStatus) {
  if (!props.canManage) {
    return;
  }
  emit("select", source);
}
</script>

<template>
  <div class="grid gap-2">
    <AppTableToolbar
      :count-label="t('sources.summary.total', { count: filteredSources.length })"
      search-enabled
      :search-placeholder="t('sources.table.filterPlaceholder')"
      :search-query="searchQuery"
      @update:search-query="searchQuery = $event"
    >
      <template #actions>
        <Button :class="toolSecondaryButtonClass" @click="emit('refresh')">
          {{ t("sources.refresh") }}
        </Button>
        <Button v-if="props.canManage" :class="toolPrimaryButtonClass" type="button" @click="emit('create')">
          {{ t("sources.newSource") }}
        </Button>
      </template>
    </AppTableToolbar>

    <div class="overflow-hidden rounded-[0.8rem] border border-app-border/90 bg-app-surface shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]">
      <DataTable
        :value="filteredSources"
        class="source-data-table tool-table-desktop"
        data-key="source_key"
        removable-sort
        resizable-columns
        column-resize-mode="expand"
        scrollable
        size="small"
        sort-field="source_key"
        :sort-order="1"
        state-storage="local"
        state-key="context69:table:sources:v2"
        table-style="width: 100%"
        @row-click="handleRowSelect($event.data)"
      >
        <template #empty>
          <div class="py-8 text-center text-sm text-app-text-dim">
            {{ t("sources.emptyMessage") }}
          </div>
        </template>

        <Column
          :header="t('sources.table.source')"
          field="source_key"
          sortable
          header-class="source-table-header-nowrap"
          body-class="align-top"
          style="min-width: 22rem"
        >
          <template #body="{ data }">
            <div class="grid gap-3 py-1.5">
              <div class="grid gap-2">
                <span class="text-base leading-6 font-semibold text-app-text">{{ data.display_name }}</span>
                <span v-if="data.display_name !== data.source_key" class="text-xs leading-5 text-app-text-dim">
                  {{ data.source_key }}
                </span>
                <div class="flex flex-wrap gap-1">
                  <Tag class="tool-chip" :value="data.connector_type" severity="secondary" />
                  <Tag class="tool-chip" :value="data.connection" severity="secondary" />
                  <Tag class="tool-chip" :value="data.sync_strategy" severity="secondary" />
                </div>
              </div>
              <p v-if="data.description" class="text-sm leading-6 text-app-text-muted">
                {{ data.description }}
              </p>
              <p class="text-sm leading-6 text-app-text-muted">
                {{ t("sources.table.summary", { batchSize: data.batch_size }) }}
              </p>
              <div v-if="(data.example_queries ?? []).length > 0" class="flex flex-wrap gap-1">
                <Tag
                  v-for="query in (data.example_queries ?? []).slice(0, 3)"
                  :key="query"
                  class="tool-chip"
                  :value="query"
                  severity="contrast"
                />
              </div>
              <div class="flex flex-wrap items-center gap-2">
                <Tag class="tool-chip" :value="originLabel(data)" :severity="originSeverity(data)" />
                <span v-if="data.origin_message" class="break-words text-sm leading-6 text-app-text-muted">{{ data.origin_message }}</span>
              </div>

              <dl class="grid gap-2 text-xs text-app-text-dim lg:hidden">
                <div class="flex items-start justify-between gap-3">
                  <dt>{{ t("sources.table.batchSize") }}</dt>
                  <dd class="whitespace-nowrap tabular-nums text-sm text-app-text">{{ data.batch_size }}</dd>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <dt>{{ t("sources.table.lastSuccess") }}</dt>
                  <dd class="text-right">{{ formatTimestamp(data.last_success_at) }}</dd>
                </div>
                <div class="grid gap-1">
                  <dt>{{ t("sources.table.cursor") }}</dt>
                  <dd class="break-all text-sm leading-6 text-app-text-muted">
                    {{ data.last_cursor_external_id ?? formatTimestamp(data.last_cursor_updated_at) }}
                  </dd>
                </div>
              </dl>
            </div>
          </template>
        </Column>

        <Column
          :header="t('sources.table.status')"
          field="last_success_at"
          sortable
          header-class="source-table-header-nowrap hidden lg:table-cell"
          body-class="hidden lg:table-cell align-top"
          style="min-width: 14rem"
        >
          <template #body="{ data }">
            <div class="grid gap-3">
              <div class="flex items-start justify-between gap-3">
                <span class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">
                  {{ t("sources.table.batchSize") }}
                </span>
                <span class="whitespace-nowrap tabular-nums text-sm text-app-text">{{ data.batch_size }}</span>
              </div>
              <div class="grid gap-0.5">
                <span class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">
                  {{ t("sources.table.lastSuccess") }}
                </span>
                <span class="whitespace-nowrap text-sm text-app-text-muted">
                  {{ formatTimestamp(data.last_success_at) }}
                </span>
              </div>
            </div>
          </template>
        </Column>

        <Column
          :header="t('sources.table.cursor')"
          field="last_cursor_updated_at"
          sortable
          header-class="source-table-header-nowrap hidden xl:table-cell"
          body-class="hidden xl:table-cell align-top"
          style="min-width: 18rem"
        >
          <template #body="{ data }">
            <div class="grid gap-1.5">
              <p class="whitespace-nowrap text-sm text-app-text-muted">
                {{ formatTimestamp(data.last_cursor_updated_at) }}
              </p>
              <p class="break-all text-sm leading-6 text-app-text-muted">
                {{ data.last_cursor_external_id ?? "--" }}
              </p>
            </div>
          </template>
        </Column>

        <Column
          v-if="props.canManage"
          :header="t('sources.table.action')"
          class="w-56"
          header-class="source-table-header-nowrap"
          body-class="align-top"
          style="min-width: 12rem"
        >
          <template #body="{ data }">
            <div class="flex flex-wrap justify-start gap-1 text-sm xl:justify-end">
              <Button :class="compactTableActionButtonClass" type="button" @click.stop="emit('edit', data)">
                {{ t("common.edit") }}
              </Button>
              <Button
                :class="compactTableActionButtonClass"
                type="button"
                :disabled="deletingMap[data.source_key]"
                @click.stop="emit('delete', data.source_key)"
              >
                {{ deletingMap[data.source_key] ? t("sources.deleting") : t("common.delete") }}
              </Button>
              <Button
                :class="compactTableActionButtonClass"
                type="button"
                :disabled="syncingMap[data.source_key] || deletingMap[data.source_key]"
                @click.stop="emit('sync', data.source_key)"
              >
                {{ syncingMap[data.source_key] ? t("sources.syncing") : t("sources.sync") }}
              </Button>
            </div>
          </template>
        </Column>
      </DataTable>

      <div class="tool-card-list source-card-list">
        <div v-if="filteredSources.length === 0" class="tool-empty">
          {{ t("sources.emptyMessage") }}
        </div>
        <article v-for="source in filteredSources" :key="source.source_key" class="tool-card">
          <div class="tool-card-header">
            <div class="min-w-0">
              <h3 class="tool-card-title">{{ source.display_name }}</h3>
              <p v-if="source.display_name !== source.source_key" class="text-xs leading-5 text-app-text-dim">
                {{ source.source_key }}
              </p>
              <div class="tool-chip-row">
                <Tag class="tool-chip" :value="source.connector_type" severity="secondary" />
                <Tag class="tool-chip" :value="source.connection" severity="secondary" />
                <Tag class="tool-chip" :value="source.sync_strategy" severity="secondary" />
              </div>
            </div>
          </div>

          <p v-if="source.description" class="tool-card-snippet max-w-[44rem]">
            {{ source.description }}
          </p>
          <p class="tool-card-snippet max-w-[44rem]" :title="source.base_query">{{ source.base_query }}</p>
          <div v-if="(source.example_queries ?? []).length > 0" class="tool-chip-row">
            <Tag
              v-for="query in (source.example_queries ?? []).slice(0, 3)"
              :key="query"
              class="tool-chip"
              :value="query"
              severity="contrast"
            />
          </div>

          <dl class="tool-meta-grid">
            <div>
              <dt>{{ t("sources.table.batchSize") }}</dt>
              <dd>{{ source.batch_size }}</dd>
            </div>
            <div>
              <dt>{{ t("sources.table.lastSuccess") }}</dt>
              <dd>{{ formatTimestamp(source.last_success_at) }}</dd>
            </div>
            <div class="tool-meta-full">
              <dt>{{ t("sources.table.cursor") }}</dt>
              <dd>{{ source.last_cursor_external_id ?? formatTimestamp(source.last_cursor_updated_at) }}</dd>
            </div>
            <div class="tool-meta-full">
              <dt>{{ t("sources.table.origin") }}</dt>
              <dd>
                <Tag class="tool-chip" :value="originLabel(source)" :severity="originSeverity(source)" />
              </dd>
            </div>
          </dl>

          <p v-if="source.origin_message" class="tool-card-snippet">
            {{ source.origin_message }}
          </p>

          <div v-if="props.canManage" class="tool-card-actions">
            <Button :class="compactTableActionButtonClass" type="button" @click="emit('edit', source)">
              {{ t("common.edit") }}
            </Button>
            <Button
              :class="compactTableActionButtonClass"
              type="button"
              :disabled="deletingMap[source.source_key]"
              @click="emit('delete', source.source_key)"
            >
              {{ deletingMap[source.source_key] ? t("sources.deleting") : t("common.delete") }}
            </Button>
            <Button
              :class="compactTableActionButtonClass"
              type="button"
              :disabled="syncingMap[source.source_key] || deletingMap[source.source_key]"
              @click="emit('sync', source.source_key)"
            >
              {{ syncingMap[source.source_key] ? t("sources.syncing") : t("sources.sync") }}
            </Button>
          </div>
        </article>
      </div>
    </div>
  </div>
</template>
