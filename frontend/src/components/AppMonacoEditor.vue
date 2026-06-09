<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

import { useUiPreferences } from "../composables/use-ui-preferences";

const props = withDefaults(defineProps<{
  modelValue: string;
  language?: string;
  inputId?: string;
  placeholder?: string;
}>(), {
  language: "plaintext",
  inputId: undefined,
  placeholder: "",
});

const emit = defineEmits<{
  "update:modelValue": [string];
}>();

const containerRef = ref<HTMLElement | null>(null);
const preferences = useUiPreferences();
let editor: any | null = null;
let isApplyingExternalValue = false;

self.MonacoEnvironment = {
  getWorker() {
    return new editorWorker();
  },
};

onMounted(() => {
  if (!containerRef.value) {
    return;
  }

  monaco.editor.setTheme(preferences.state.theme === "dark" ? "vs-dark" : "vs");
  editor = monaco.editor.create(containerRef.value, {
    value: props.modelValue,
    language: props.language,
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    wordWrap: "on",
    fontSize: 13,
    lineHeight: 22,
    padding: {
      top: 14,
      bottom: 14,
    },
    tabSize: 2,
    insertSpaces: true,
    suggest: {
      showWords: false,
    },
    quickSuggestions: false,
  });

  if (props.placeholder) {
    editor.updateOptions({
      placeholder: props.placeholder,
    });
  }

  editor.onDidChangeModelContent(() => {
    if (!editor || isApplyingExternalValue) {
      return;
    }
    emit("update:modelValue", editor.getValue());
  });
});

watch(
  () => preferences.state.theme,
  (theme) => {
    monaco.editor.setTheme(theme === "dark" ? "vs-dark" : "vs");
  },
);

watch(
  () => props.modelValue,
  (value) => {
    if (!editor || editor.getValue() === value) {
      return;
    }

    isApplyingExternalValue = true;
    editor.setValue(value);
    isApplyingExternalValue = false;
  },
);

watch(
  () => props.language,
  (language) => {
    if (!editor) {
      return;
    }
    const model = editor.getModel();
    if (model) {
      monaco.editor.setModelLanguage(model, language);
    }
  },
);

onBeforeUnmount(() => {
  editor?.dispose();
  editor = null;
});
</script>

<template>
  <div class="app-monaco-shell">
    <div :id="inputId" ref="containerRef" class="app-monaco-editor" />
  </div>
</template>
