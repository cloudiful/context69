import type { GitProviderKind } from "@/services/api";

export const PROVIDERS: GitProviderKind[] = ["github", "forgejo"];

export function required(value: string, message: string): string | undefined {
  return value.trim().length > 0 ? undefined : message;
}
