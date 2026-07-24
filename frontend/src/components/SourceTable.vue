<script setup lang="ts">
import { computed } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import type { SourceStatus } from "../services/api";
import TablePagination from "./TablePagination.vue";
import { formatTimestamp } from "../utils/format";

const props = withDefaults(defineProps<{
  sources: SourceStatus[];
  page: number;
  pageSize: number;
  total: number;
  query: string;
  loading?: boolean;
  syncingMap: Record<string, boolean>;
  deletingMap: Record<string, boolean>;
  canManage?: boolean;
}>(), {
  canManage: true,
});

const emit = defineEmits<{
  create: [];
  refresh: [];
  page: [number];
  "page-size": [number];
  "update:query": [string];
  sync: [string];
  edit: [SourceStatus];
  delete: [string];
  select: [SourceStatus];
}>();

const { t } = useI18n();
function originSeverity(source: SourceStatus) {
  if (source.origin_status === "connected") {
    return "success";
  }
  if (source.origin_status === "unreachable" || source.origin_status === "misconfigured") {
    return "error";
  }
  return "neutral";
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

const columns = computed<TableColumn<SourceStatus>[]>(() => [
  { accessorKey: "source_key", header: t("sources.table.source") },
  { id: "status", header: t("sources.table.status"), meta: { class: { th: "hidden lg:table-cell", td: "hidden lg:table-cell align-top" } } },
  { id: "cursor", header: t("sources.table.cursor"), meta: { class: { th: "hidden xl:table-cell", td: "hidden xl:table-cell align-top" } } },
  ...(props.canManage ? [{ id: "actions", header: t("sources.table.action") }] : []),
]);

function selectRow(_event: Event, row: { original: SourceStatus }) {
  handleRowSelect(row.original);
}
</script>

<template>
  <div class="grid gap-2">
    <UDashboardToolbar class="flex-wrap justify-between gap-2">
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <UBadge :label="t('sources.summary.total', { count: total })" color="neutral" variant="subtle" />
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <UInput
          :model-value="query"
          class="w-full min-w-0 md:w-72"
          icon="i-lucide-search"
          :placeholder="t('sources.table.filterPlaceholder')"
          @update:model-value="emit('update:query', $event)"
        />
        <UButton color="neutral" variant="outline" @click="emit('refresh')">
          {{ t("sources.refresh") }}
        </UButton>
        <UButton v-if="props.canManage" type="button" @click="emit('create')">
          {{ t("sources.newSource") }}
        </UButton>
      </div>
    </UDashboardToolbar>

    <UTable
        class="min-w-0 max-w-full"
        :data="sources"
        :columns="columns"
        :loading="loading"
        @select="selectRow"
      >
        <template #empty>
          <div class="py-8 text-center text-sm text-muted-color">
            {{ t("sources.emptyMessage") }}
          </div>
        </template>

        <template #source_key-cell="{ row }">
          <template v-if="row.original">
            <div class="grid gap-3 py-1.5">
              <div class="grid gap-2">
                <span class="text-base leading-6 font-semibold">{{ row.original.display_name }}</span>
                <span v-if="row.original.display_name !== row.original.source_key" class="text-xs leading-5 text-muted">{{ row.original.source_key }}
                </span>
                <div class="flex flex-wrap gap-1">
                  <UBadge :label="row.original.connector_type" color="neutral" variant="subtle" />
                  <UBadge :label="row.original.connection" color="neutral" variant="subtle" />
                  <UBadge :label="row.original.sync_strategy" color="neutral" variant="subtle" />
                </div>
              </div>
              <p v-if="row.original.description" class="text-sm leading-6 text-muted">{{ row.original.description }}
              </p>
              <p class="text-sm leading-6 text-muted-color">
                {{ t("sources.table.summary", { batchSize: row.original.batch_size }) }}
              </p>
              <div v-if="(row.original.example_queries ?? []).length > 0" class="flex flex-wrap gap-1">
                <UBadge
                  v-for="query in (row.original.example_queries ?? []).slice(0, 3)"
                  :key="query"
                  :label="query"
                  color="neutral"
                />
              </div>
              <div class="flex flex-wrap items-center gap-2">
                <UBadge :label="originLabel(row.original)" :color="originSeverity(row.original)" variant="subtle" />
                <span v-if="row.original.origin_message" class="break-words text-sm leading-6 text-muted">{{ row.original.origin_message }}</span>
              </div>

              <dl class="grid gap-2 text-xs text-muted-color lg:hidden">
                <div class="flex items-start justify-between gap-3">
                  <dt>{{ t("sources.table.batchSize") }}</dt>
                  <dd class="whitespace-nowrap tabular-nums text-sm">{{ row.original.batch_size }}</dd>
                </div>
                <div class="flex items-start justify-between gap-3">
                  <dt>{{ t("sources.table.lastSuccess") }}</dt>
                  <dd class="text-right">{{ formatTimestamp(row.original.last_success_at) }}</dd>
                </div>
                <div class="grid gap-1">
                  <dt>{{ t("sources.table.cursor") }}</dt>
                  <dd class="break-all text-sm leading-6 text-muted-color">
                    {{ row.original.last_cursor_external_id ?? formatTimestamp(row.original.last_cursor_updated_at) }}
                  </dd>
                </div>
              </dl>
            </div>
          </template>
        </template>

        <template #status-cell="{ row }">
            <div class="grid gap-3">
              <div class="flex items-start justify-between gap-3">
                <span class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-muted-color">
                  {{ t("sources.table.batchSize") }}
                </span>
                <span class="whitespace-nowrap tabular-nums text-sm">{{ row.original.batch_size }}</span>
              </div>
              <div class="grid gap-0.5">
                <span class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-muted-color">
                  {{ t("sources.table.lastSuccess") }}
                </span>
                <span class="whitespace-nowrap text-sm text-muted-color">
                  {{ formatTimestamp(row.original.last_success_at) }}
                </span>
              </div>
            </div>
        </template>

        <template #cursor-cell="{ row }">
            <div class="grid gap-1.5">
              <p class="whitespace-nowrap text-sm text-muted-color">
                {{ formatTimestamp(row.original.last_cursor_updated_at) }}
              </p>
              <p class="break-all text-sm leading-6 text-muted-color">
                {{ row.original.last_cursor_external_id ?? "--" }}
              </p>
            </div>
        </template>

        <template #actions-cell="{ row }">
            <div class="flex flex-wrap justify-start gap-1 text-sm xl:justify-end">
              <UButton size="sm" variant="ghost" color="neutral" @click.stop="emit('edit', row.original)">
                {{ t("common.edit") }}
              </UButton>
              <UButton
                size="sm"
      variant="ghost"                color="error"
                type="button"
                :disabled="deletingMap[row.original.source_key]"
                @click.stop="emit('delete', row.original.source_key)"
              >
                {{ deletingMap[row.original.source_key] ? t("sources.deleting") : t("common.delete") }}
              </UButton>
              <UButton
                size="sm"
      variant="ghost"                color="neutral"
                type="button"
                :disabled="syncingMap[row.original.source_key] || deletingMap[row.original.source_key]"
                @click.stop="emit('sync', row.original.source_key)"
              >
                {{ syncingMap[row.original.source_key] ? t("sources.syncing") : t("sources.sync") }}
              </UButton>
            </div>
        </template>
    </UTable>

    <TablePagination
      :page="page"
      :page-size="pageSize"
      :total="total"
      @update:page="emit('page', $event)"
      @update:page-size="emit('page-size', $event)"
    />
  </div>
</template>
