<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import AppTextField from "../components/AppTextField.vue";
import { AuthError, authSessionState, login } from "../services/auth";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const loginName = ref("");
const password = ref("");
const errorMessage = ref("");
const busy = ref(false);

const sessionReasonMessage = computed(() => {
  const reason = route.query.reason;
  if (reason === "expired") {
    return t("auth.sessionExpired");
  }
  if (reason === "network") {
    return t("auth.sessionRestoreFailed");
  }
  return "";
});

const redirectTarget = computed(() => {
  const redirect = route.query.redirect;
  return typeof redirect === "string" && redirect ? redirect : "/search";
});

async function submit() {
  if (busy.value) {
    return;
  }

  errorMessage.value = "";
  busy.value = true;

  try {
    await login({
      login_name: loginName.value.trim(),
      password: password.value,
    });
    await router.replace(redirectTarget.value);
  } catch (error) {
    if (error instanceof AuthError) {
      if (error.reason === "invalid_credentials") {
        errorMessage.value = t("auth.invalidCredentials");
      } else if (error.reason === "network") {
        errorMessage.value = t("auth.networkError");
      } else {
        errorMessage.value = error.message || t("auth.loginFailed");
      }
    } else {
      errorMessage.value = t("auth.loginFailed");
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="auth-page-shell">
    <section class="auth-hero">
      <div class="auth-hero-mark">C69</div>
      <div class="auth-hero-copy">
        <p class="section-label">{{ t("auth.label") }}</p>
        <h1 class="auth-hero-title">Context69</h1>
        <p class="auth-hero-text">{{ t("auth.description") }}</p>
      </div>

      <div class="auth-hero-tags" aria-label="Available workspaces">
        <span class="auth-hero-tag">{{ t("nav.search") }}</span>
        <span class="auth-hero-tag">{{ t("nav.groups") }}</span>
        <span class="auth-hero-tag">{{ t("nav.settings") }}</span>
      </div>

      <div v-if="authSessionState.user" class="auth-hero-user">
        <span class="auth-session-caption">{{ authSessionState.user.display_name }}</span>
        <span class="text-xs text-app-text-dim">{{ authSessionState.user.login_name }}</span>
      </div>
    </section>

    <AppPanel class="auth-panel" :title="t('auth.title')" :label="t('auth.label')">
      <AppStateMessage
        v-if="sessionReasonMessage"
        severity="warn"
        :title="t('auth.sessionStateTitle')"
      >
        {{ sessionReasonMessage }}
      </AppStateMessage>

      <AppStateMessage
        v-if="errorMessage"
        severity="error"
        :title="t('auth.loginFailedTitle')"
      >
        {{ errorMessage }}
      </AppStateMessage>

      <form class="auth-form" @submit.prevent="submit">
        <AppTextField
          input-id="login-name"
          :label="t('auth.loginName')"
          autocomplete="username"
          :model-value="loginName"
          @update:model-value="loginName = $event"
        />
        <AppTextField
          input-id="login-password"
          :label="t('auth.password')"
          type="password"
          autocomplete="current-password"
          :model-value="password"
          @update:model-value="password = $event"
        />

        <div class="auth-form-actions">
          <Button
            class="app-primary-button auth-submit-button"
            type="submit"
            :disabled="busy"
            :loading="busy"
          >
            {{ busy ? t("auth.signingIn") : t("auth.signIn") }}
          </Button>
        </div>
      </form>
    </AppPanel>
  </div>
</template>
