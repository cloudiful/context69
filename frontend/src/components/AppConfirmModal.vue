<script setup lang="ts">
const props = withDefaults(defineProps<{
  title: string;
  message: string;
  acceptLabel?: string;
  rejectLabel?: string;
}>(), {
  acceptLabel: "Confirm",
  rejectLabel: "Cancel",
});

const emit = defineEmits<{ close: [accepted: boolean] }>();
</script>

<template>
  <UModal default-open :title="props.title" :description="props.message" @update:open="!$event && emit('close', false)">
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton color="neutral" variant="outline" :label="props.rejectLabel" @click="emit('close', false)" />
        <UButton color="error" :label="props.acceptLabel" @click="emit('close', true)" />
      </div>
    </template>
  </UModal>
</template>
