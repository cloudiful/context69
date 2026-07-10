<script setup lang="ts">
import Tree from "primevue/tree";
import type { TreeNode } from "primevue/treenode";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import type { FolderTreeNode } from "../types/library";
import type { LibraryFolderNode } from "../services/api";

const props = defineProps<{
  expandedKeys: Record<string, boolean>;
  selectionKeys: Record<string, boolean>;
  treeLoading: boolean;
  treeNodes: FolderTreeNode[];
}>();

const emit = defineEmits<{
  "node-contextmenu": [MouseEvent, LibraryFolderNode];
  "node-select": [TreeNode];
  "update:expandedKeys": [Record<string, boolean>];
}>();
</script>

<template>
  <div class="library-pane flex h-full min-h-[42rem] flex-col">
    <div class="min-h-0 flex-1 overflow-auto">
      <AsyncStateBlock
        :loading="props.treeLoading"
        :loading-title="$t('common.loading')"
        :loading-message="$t('library.loadingTree')"
        :empty="!props.treeLoading && props.treeNodes.length === 0"
        :empty-title="$t('library.emptyFolderTitle')"
        :empty-message="$t('library.emptyFolderMessage')"
        empty-variant="soft"
      >
        <Tree
          :expandedKeys="props.expandedKeys"
          :selectionKeys="props.selectionKeys"
          :meta-key-selection="false"
          :value="props.treeNodes"
          selection-mode="single"
          class="w-full"
          @update:expandedKeys="emit('update:expandedKeys', $event)"
          @node-select="emit('node-select', $event)"
        >
          <template #default="{ node }">
            <div
              class="min-w-0 rounded-2xl px-2.5 py-2 transition hover:bg-app-surface-soft/70"
              @contextmenu.prevent.stop="emit('node-contextmenu', $event, node.data.folder)"
            >
              <div class="min-w-0">
                <p class="truncate text-sm font-semibold leading-6 text-app-text">{{ node.label }}</p>
                <p class="truncate text-xs leading-5 text-app-text-dim">{{ node.data.countsLabel }}</p>
              </div>
            </div>
          </template>
        </Tree>
      </AsyncStateBlock>
    </div>
  </div>
</template>
