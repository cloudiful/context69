<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { en, zh_cn } from "@nuxt/ui/locale";

import AppMobileNav from "./components/AppMobileNav.vue";
import AppRouteBreadcrumbs from "./components/AppRouteBreadcrumbs.vue";
import AppSidebar from "./components/AppSidebar.vue";
import { useUiPreferences } from "./composables/use-ui-preferences";
import { authSessionState, restoreSession } from "./services/auth/session";

const preferences = useUiPreferences();
const { locale } = useI18n();
const uiLocale = computed(() => locale.value === "zh-CN" ? zh_cn : en);
const appReady = computed(() => authSessionState.ready);
const route = useRoute();
const isLoginRoute = computed(() => route.name === "login");
const showGlobalRouteBar = computed(() => {
  const routeName = String(route.name ?? "");
  return routeName === "group-overview"
    || routeName === "group-members"
    || routeName === "group-settings"
    || routeName.startsWith("settings-")
    || routeName === "processing-queue";
});
const isSettingsRoute = computed(() => String(route.name ?? "").startsWith("settings-"));
const fillsRouteContent = computed(() => route.meta.contentLayout === "fill");

async function retrySessionRestore() {
  if (await restoreSession()) window.location.reload();
}

onMounted(() => {
  preferences.hydrate();
});

watch(locale, (value) => {
  document.documentElement.lang = value === "zh-CN" ? "zh-CN" : "en";
}, { immediate: true });
</script>

<template>
  <UApp :locale="uiLocale">
  <main v-if="isLoginRoute" class="flex min-h-screen min-w-0 flex-col bg-surface-0 text-color dark:bg-surface-950">
    <RouterView />
  </main>

    <UDashboardGroup v-else class="min-h-screen">
      <AppSidebar />
      <UDashboardPanel class="min-w-0">
        <main
          :class="[
            'flex min-h-0 min-w-0 flex-1 flex-col px-2 py-2 pb-20 md:px-3 md:pb-3',
            isSettingsRoute
              ? 'h-screen overflow-hidden'
              : fillsRouteContent
                ? 'md:h-screen md:overflow-hidden'
                : 'min-h-screen',
          ]"
        >
      <div
        v-if="authSessionState.restoreError && !authSessionState.user"
        class="grid flex-1 place-items-center px-4"
      >
        <div class="grid max-w-md justify-items-center gap-3 text-center">
          <UAlert
            color="error"
            variant="subtle"
            :title="$t('auth.sessionUnavailable')"
            :description="$t('auth.sessionUnavailableMessage')"
          />
          <UButton size="sm" @click="retrySessionRestore">
            <UIcon name="i-lucide-refresh-cw" />
            <span>{{ $t("common.retry") }}</span>
          </UButton>
        </div>
      </div>
      <div
        v-else-if="appReady"
        class="grid min-h-0 min-w-0 flex-1 gap-2"
        :class="showGlobalRouteBar ? 'grid-rows-[auto_minmax(0,1fr)]' : 'grid-rows-[minmax(0,1fr)]'"
      >
        <AppRouteBreadcrumbs v-if="showGlobalRouteBar" />
        <div :class="fillsRouteContent ? 'h-full min-h-0 min-w-0' : 'min-w-0 self-start'">
          <RouterView />
        </div>
      </div>
        </main>
        <AppMobileNav />
      </UDashboardPanel>
    </UDashboardGroup>
  </UApp>
</template>
