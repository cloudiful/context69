<script setup lang="ts">
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AppStateMessage from "../components/AppStateMessage.vue";

const router = useRouter();
const { t } = useI18n();

function returnToSearch() {
  void router.push("/login");
}
</script>

<template>
  <div class="mx-auto max-w-2xl py-16">
    <UCard>
        <div class="grid gap-6 text-center">
          <div>
            <p>404</p>
            <h1>{{ t("notFound.title") }}</h1>
          </div>

          <AppStateMessage color="neutral">
            {{ t("notFound.message") }}
          </AppStateMessage>

          <div class="flex justify-center">
            <UButton @click="returnToSearch">{{ t("notFound.action") }}</UButton>
          </div>
        </div>
    </UCard>
  </div>
</template>
