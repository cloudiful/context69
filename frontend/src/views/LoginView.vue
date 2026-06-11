<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { Form, FormField } from "@primevue/forms";
import { zodResolver } from "@primevue/forms/resolvers/zod";
import Button from "primevue/button";
import Fluid from "primevue/fluid";
import InputText from "primevue/inputtext";
import Message from "primevue/message";
import * as z from "zod";

import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import { AuthError, authSessionState, login } from "../services/auth";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const errorMessage = ref("");
const busy = ref(false);

const initialValues = {
  login_name: "",
  password: "",
};

const resolver = computed(() =>
  zodResolver(
    z.object({
      login_name: z.string().trim().min(1, { message: t("auth.validation.loginNameRequired") }),
      password: z.string().min(1, { message: t("auth.validation.passwordRequired") }),
    }),
  ),
);

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

async function submit(event: { valid: boolean; values: Record<string, unknown> }) {
  if (busy.value) {
    return;
  }

  if (!event.valid) {
    return;
  }

  errorMessage.value = "";
  busy.value = true;

  try {
    await login({
      login_name: String(event.values.login_name ?? "").trim(),
      password: String(event.values.password ?? ""),
    });
    await router.replace(redirectTarget.value);
  } catch (error) {
    const authError = readAuthError(error);
    if (authError) {
      if (authError.reason === "invalid_credentials") {
        errorMessage.value = t("auth.invalidCredentials");
      } else if (authError.reason === "network") {
        errorMessage.value = t("auth.networkError");
      } else {
        errorMessage.value = authError.message || t("auth.loginFailed");
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
    <AppPanel class="auth-panel" :title="t('auth.title')">
      <div v-if="authSessionState.user" class="auth-session-inline">
        <span class="auth-session-inline-name">{{ authSessionState.user.display_name }}</span>
        <span class="text-xs text-app-text-dim">{{ authSessionState.user.login_name }}</span>
      </div>

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

      <Fluid>
        <Form
          class="auth-form"
          :initial-values="initialValues"
          :resolver="resolver"
          @submit="submit"
        >
          <FormField v-slot="$field" name="login_name" :initial-value="initialValues.login_name">
            <label class="app-form-field auth-field">
              <span class="app-form-field-label">{{ t("auth.loginName") }}</span>
              <InputText
                id="login-name"
                v-bind="$field.props"
                :model-value="$field.value"
                :disabled="busy"
                :placeholder="t('auth.loginNamePlaceholder')"
                autocomplete="username"
                fluid
                @update:model-value="$field.props.onInput({ value: $event })"
              />
              <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                {{ $field.error?.message }}
              </Message>
            </label>
          </FormField>

          <FormField v-slot="$field" name="password" :initial-value="initialValues.password">
            <label class="app-form-field auth-field">
              <span class="app-form-field-label">{{ t("auth.password") }}</span>
              <InputText
                id="login-password"
                v-bind="$field.props"
                :model-value="$field.value"
                type="password"
                :disabled="busy"
                :placeholder="t('auth.passwordPlaceholder')"
                autocomplete="current-password"
                fluid
                @update:model-value="$field.props.onInput({ value: $event })"
              />
              <Message v-if="$field.invalid" severity="error" size="small" variant="simple">
                {{ $field.error?.message }}
              </Message>
            </label>
          </FormField>

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
        </Form>
      </Fluid>
    </AppPanel>
  </div>
</template>
