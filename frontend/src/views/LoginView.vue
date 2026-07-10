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
import { useErrorToast } from "../composables/use-error-toast";
import { AuthError, authSessionState, login } from "../services/auth/session";
import { authSubmitButtonClass } from "../ui/button-classes";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();

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
  <div class="auth-page-shell">
    <AppPanel class="auth-panel" :title="t('auth.title')">
      <div v-if="authSessionState.user" class="auth-session-inline">
        <span class="auth-session-inline-name">{{ authSessionState.user.display_name }}</span>
        <span class="text-xs text-app-text-dim">{{ authSessionState.user.login_name }}</span>
      </div>

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
              unstyled
              :class="authSubmitButtonClass"
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
