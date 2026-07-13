<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import type { FormSubmitEvent } from "@nuxt/ui";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import * as z from "zod";

import AppPanel from "../components/AppPanel.vue";
import { useErrorToast } from "../composables/use-error-toast";
import { AuthError, authSessionState, login } from "../services/auth/session";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();

const busy = ref(false);

const state = reactive({
  login_name: "",
  password: "",
});

const schema = computed(() => z.object({
  login_name: z.string().trim().min(1, { message: t("auth.validation.loginNameRequired") }),
  password: z.string().min(1, { message: t("auth.validation.passwordRequired") }),
}));

const redirectTarget = computed(() => {
  const redirect = route.query.redirect;
  return typeof redirect === "string" && redirect ? redirect : "/search";
});

function readAuthError(error: unknown) {
  if (error instanceof AuthError) {
    return error;
  }

  if (error && typeof error === "object" && "reason" in error) {
    const reason = typeof error.reason === "string" ? error.reason : "unknown";
    const message = "message" in error && typeof error.message === "string"
      ? error.message
      : t("auth.loginFailed");
    return new AuthError(message, 0, reason as AuthError["reason"]);
  }

  return null;
}

async function submit(event: FormSubmitEvent<z.output<typeof schema.value>>) {
  if (busy.value) {
    return;
  }

  busy.value = true;

  try {
    await login({
      login_name: event.data.login_name.trim(),
      password: event.data.password,
    });
    await router.replace(redirectTarget.value);
  } catch (error) {
    const authError = readAuthError(error);
    if (authError) {
      if (authError.reason === "invalid_credentials") {
        showErrorToast(null, t("auth.invalidCredentials"));
      } else if (authError.reason === "network") {
        showErrorToast(null, t("auth.networkError"));
      } else {
        showErrorToast(authError, t("auth.loginFailed"));
      }
    } else {
      showErrorToast(error, t("auth.loginFailed"));
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="flex min-h-[calc(100vh-5rem)] w-full items-center justify-center px-3 py-6">
    <AppPanel class="w-full max-w-[24rem]" :title="t('auth.title')">
      <div v-if="authSessionState.user" class="grid gap-1 rounded-xl border border-surface bg-emphasis px-3 py-2">
        <span class="text-sm font-semibold text-color">{{ authSessionState.user.display_name }}</span>
        <span class="text-xs text-muted-color">{{ authSessionState.user.login_name }}</span>
      </div>

      <div>
        <UForm
          class="grid gap-3"
          :state="state"
          :schema="schema"
          @submit="submit"
        >
          <UFormField name="login_name" :label="t('auth.loginName')">
            <UInput id="login-name" v-model="state.login_name" class="w-full" :disabled="busy" :placeholder="t('auth.loginNamePlaceholder')" autocomplete="username" />
          </UFormField>

          <UFormField name="password" :label="t('auth.password')">
            <UInput id="login-password" v-model="state.password" class="w-full" type="password" :disabled="busy" :placeholder="t('auth.passwordPlaceholder')" autocomplete="current-password" />
          </UFormField>

          <div class="pt-1">
            <UButton
              class="w-full"
              type="submit"
              :disabled="busy"
              :aria-busy="busy"
            >
              <UIcon v-if="busy" name="i-lucide-loader-circle" class="h-4 w-4 animate-spin" />
              {{ busy ? t("auth.signingIn") : t("auth.signIn") }}
            </UButton>
          </div>
        </UForm>
      </div>
    </AppPanel>
  </div>
</template>
