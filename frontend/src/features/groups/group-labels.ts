import type { GroupKind, MembershipRole } from "@/services/api";

type Translate = (key: string, params?: Record<string, unknown>) => string;

const GROUP_KIND_VALUES: readonly GroupKind[] = ["personal", "shared"];
const MEMBERSHIP_ROLE_VALUES: readonly MembershipRole[] = ["owner", "maintainer", "viewer"];

/**
 * Known generated enum values get their localized label; a value this build does
 * not know stays visible as its raw code instead of crashing or silently
 * implying a role. Display text never feeds permission logic: gates keep reading
 * the typed role code. Visibility lives in the neutral `@/lib/visibility-label`
 * helper because the library and document reader share it.
 */
function localizedLabel(
  t: Translate,
  prefix: string,
  value: string | null | undefined,
  knownValues: readonly string[],
): string {
  if (!value) {
    return "--";
  }
  return knownValues.includes(value) ? t(`${prefix}.${value}`) : value;
}

export function groupKindLabel(t: Translate, kind: GroupKind | null | undefined): string {
  return localizedLabel(t, "groups.kindOptions", kind, GROUP_KIND_VALUES);
}

export function membershipRoleLabel(
  t: Translate,
  role: MembershipRole | null | undefined,
): string {
  return localizedLabel(t, "groups.roleOptions", role, MEMBERSHIP_ROLE_VALUES);
}
