import type { MembershipRole } from "@/services/api";

const ROLE_RANK: Record<MembershipRole, number> = {
  viewer: 1,
  maintainer: 2,
  owner: 3,
};

/**
 * Group permissions are default-deny: an absent role has rank 0, so a group the
 * caller cannot read never grants member administration or deletion rights.
 */
export function roleRank(role: MembershipRole | null | undefined): number {
  return role ? ROLE_RANK[role] ?? 0 : 0;
}

export function canManageGroup(role: MembershipRole | null | undefined): boolean {
  return roleRank(role) >= ROLE_RANK.maintainer;
}

export function canOwnGroup(role: MembershipRole | null | undefined): boolean {
  return roleRank(role) >= ROLE_RANK.owner;
}

export function isDescendantPath(
  candidatePath: string | null | undefined,
  parentPath: string,
): boolean {
  return !!candidatePath && (candidatePath === parentPath || candidatePath.startsWith(`${parentPath}/`));
}

export function groupPathOf(group: { group_path?: string | null; group_key: string }): string {
  return group.group_path ?? group.group_key;
}

export function roleBadgeVariant(role: MembershipRole | null | undefined): "default" | "secondary" | "outline" {
  if (role === "owner") {
    return "default";
  }
  if (role === "maintainer") {
    return "secondary";
  }
  return "outline";
}
