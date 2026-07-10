import { authSessionState, clearSession } from "../services/auth/session";
import type { AuthUserResponse } from "../services/api";

export function setAuthenticatedUser(
  user: Partial<AuthUserResponse> = {},
  accessToken = "access-token",
) {
  authSessionState.accessToken = accessToken;
  authSessionState.user = {
    user_id: 1,
    login_name: "admin",
    display_name: "Administrator",
    is_admin: true,
    personal_group_path: "personal-admin",
    personal_group_role: "owner",
    ...user,
  };
  authSessionState.status = "authenticated";
  authSessionState.ready = true;
  authSessionState.lastFailureReason = null;
}

export function setGuest(reason: "expired" | "network" | "unknown" | null = null) {
  clearSession(reason);
  authSessionState.ready = true;
}
