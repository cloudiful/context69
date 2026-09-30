<script setup lang="ts">
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import UPage from "@nuxt/ui/components/Page.vue";
import UPageBody from "@nuxt/ui/components/PageBody.vue";

const router = useRouter();
const { t } = useI18n();

function returnToSearch() {
  void router.push("/search");
}
</script>

<template>
  <UPage class="mx-auto w-full max-w-2xl py-16">
    <UPageBody :ui="{ base: 'mt-0 pb-0' }">
      <UCard>
        <div class="grid gap-6 text-center">
          <div>
            <p>404</p>
            <h1>{{ t("notFound.title") }}</h1>
          </div>

          <UAlert color="neutral" variant="subtle" :description="t('notFound.message')" />

          <div class="flex justify-center">
            <UButton @click="returnToSearch">{{ t("notFound.action") }}</UButton>
          </div>
        </div>
      </UCard>
    </UPageBody>
  </UPage>
</template>
