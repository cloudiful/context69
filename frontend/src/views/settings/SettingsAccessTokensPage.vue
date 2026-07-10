<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsAccessTokensSection from "../../components/settings-sections/SettingsAccessTokensSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const personalAccessTokenCanCreate = computed(() => unref(state.personalAccessTokenCanCreate));
const personalAccessTokenExpiryOptions = computed(() => unref(state.personalAccessTokenExpiryOptions));
const personalAccessTokenScopeOptions = computed(() => unref(state.personalAccessTokenScopeOptions));
const personalAccessTokenScopeToggleModel = computed(() => unref(state.personalAccessTokenScopeToggleModel));
const personalAccessTokens = computed(() => unref(state.personalAccessTokens));
const personalAccessTokensCreating = computed(() => unref(state.personalAccessTokensCreating));
const personalAccessTokensLoading = computed(() => unref(state.personalAccessTokensLoading));
const personalAccessTokensReveal = computed(() => unref(state.personalAccessTokensReveal));
</script>

<template>
  <SettingsAccessTokensSection
    :create-personal-access-token="state.createPersonalAccessToken"
    :confirm-revoke-personal-access-token="state.confirmRevokePersonalAccessToken"
    :copy-personal-access-token="state.copyPersonalAccessToken"
    :dismiss-personal-access-token-reveal="state.dismissPersonalAccessTokenReveal"
    :personal-access-token-draft="state.personalAccessTokenDraft"
    :personal-access-token-can-create="personalAccessTokenCanCreate"
    :personal-access-token-expiry-options="personalAccessTokenExpiryOptions"
    :personal-access-token-scope-options="personalAccessTokenScopeOptions"
    :personal-access-token-scope-toggle-model="personalAccessTokenScopeToggleModel"
    :personal-access-tokens="personalAccessTokens"
    :personal-access-tokens-creating="personalAccessTokensCreating"
    :personal-access-tokens-loading="personalAccessTokensLoading"
    :personal-access-tokens-reveal="personalAccessTokensReveal"
    @update:personal-access-token-scope-toggle-model="state.personalAccessTokenScopeToggleModel.value = $event"
  />
</template>
