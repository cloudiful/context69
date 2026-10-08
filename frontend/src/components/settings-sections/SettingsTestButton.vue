<script setup lang="ts">
withDefaults(defineProps<{
  testing: boolean;
  testId: string;
  label: string;
  disabled?: boolean;
}>(), {
  disabled: false,
});

const emit = defineEmits<{ test: [] }>();
</script>

<template>
  <UButton
    size="sm"
    color="neutral"
    :disabled="testing || disabled"
    :aria-busy="testing"
    :data-testid="testId"
    @click="emit('test')"
  >
    <UIcon name="i-lucide-loader-circle" v-if="testing" class="h-4 w-4" />
    <UIcon v-else name="i-lucide-zap" />
    <span>{{ label }}</span>
  </UButton>
</template>
