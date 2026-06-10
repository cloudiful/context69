<script setup lang="ts">
import { useRoute } from "vue-router";
import Button from "primevue/button";

import AppMdiIcon from "./AppMdiIcon.vue";
import AppFooterTools from "./AppFooterTools.vue";
import { usePageChrome } from "../composables/use-app-navigation";
import { useUiPreferences } from "../composables/use-ui-preferences";

const route = useRoute();
const preferences = useUiPreferences();
const { pageTitle, breadcrumb } = usePageChrome();
const mdiMenu = "M3,6H21V8H3V6M3,11H21V13H3V11M3,16H21V18H3V16Z";
</script>

<template>
  <header v-if="route.name !== 'login'" class="app-contextbar">
    <div class="app-contextbar-copy">
      <Button
        class="app-control-button app-contextbar-menu md:hidden"
        type="button"
        @click="preferences.toggleMobileNav"
      >
        <AppMdiIcon :path="mdiMenu" title="Navigation" class="app-sidebar-link-icon" />
      </Button>
      <p class="app-contextbar-breadcrumb">{{ breadcrumb.join(" / ") }}</p>
      <h1 class="app-contextbar-title">{{ pageTitle }}</h1>
    </div>

    <div class="app-contextbar-actions">
      <AppFooterTools />
    </div>
  </header>
</template>
