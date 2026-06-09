import type {
  ProviderAccountResponse,
  UpsertProviderAccountRequest,
} from "../../services/api";
import type { ProviderAccountDraft } from "./settings-types";

export function isProviderDraftBlank(draft: ProviderAccountDraft) {
  return !draft.account_key.trim()
    && draft.provider_kind.trim() === "openai_compatible"
    && !draft.display_name.trim()
    && !draft.base_url.trim()
    && !draft.api_key.trim()
    && !draft.clear_api_key
    && !draft.disabled;
}

export function buildProviderAccountComparablePayload(
  draft: ProviderAccountDraft,
): UpsertProviderAccountRequest {
  return {
    account_key: draft.account_key.trim(),
    provider_kind: draft.provider_kind.trim(),
    display_name: draft.display_name.trim(),
    base_url: draft.base_url.trim(),
    clear_api_key: draft.clear_api_key,
    disabled: draft.disabled,
  };
}

export function providerResponseToPayload(
  response: ProviderAccountResponse,
): UpsertProviderAccountRequest {
  return {
    account_key: response.account_key,
    provider_kind: response.provider_kind,
    display_name: response.display_name,
    base_url: response.base_url,
    clear_api_key: false,
    disabled: !!response.disabled_at,
  };
}
