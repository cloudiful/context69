<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AppFooterTools from "./AppFooterTools.vue";
import { authSessionState, isAuthenticated, logout } from "../services/auth";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const pageTitle = computed(() => {
  if (route.name === "search") return t("nav.search");
  if (route.name === "groups" || route.name === "group-detail" || route.name === "project") return t("nav.groups");
  if (route.name === "settings") return t("nav.settings");
  if (route.name === "document") return t("document.title");
  if (route.name === "login") return t("auth.title");
  return "Context69";
});

const breadcrumb = computed(() => {
  if (route.name === "project") {
    return [t("nav.groups"), String(route.params.groupKey ?? ""), String(route.params.projectKey ?? "")].filter(Boolean);
  }
  if (route.name === "group-detail") {
    return [t("nav.groups"), String(route.params.groupKey ?? "")].filter(Boolean);
  }
  return [pageTitle.value];
});

async function signOut() {
  await logout();
  await router.replace({ name: "login" });
}
</script>

<template>
  <header v-if="route.name !== 'login'" class="app-contextbar">
    <div class="app-contextbar-copy">
      <p class="app-contextbar-breadcrumb">{{ breadcrumb.join(" / ") }}</p>
      <h1 class="app-contextbar-title">{{ pageTitle }}</h1>
    </div>

    <div class="app-contextbar-actions">
      <AppFooterTools />

      <div v-if="isAuthenticated() && authSessionState.user" class="app-contextbar-user">
        <div class="app-contextbar-user-copy">
          <span class="app-contextbar-user-name">{{ authSessionState.user.display_name }}</span>
          <span class="app-contextbar-user-meta">{{ authSessionState.user.login_name }}</span>
        </div>
        <Button class="app-control-button" type="button" @click="signOut">
          {{ t("auth.logout") }}
        </Button>
      </div>
    </div>
  </header>
</template>
