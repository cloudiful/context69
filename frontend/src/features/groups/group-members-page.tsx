import { useTranslation } from "react-i18next";

import { TablePagination } from "@/components/common/table-pagination";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { MemberSortBy } from "@/services/api";

import { membershipRoleLabel } from "./group-labels";
import { roleBadgeVariant } from "./permissions";
import { useGroupWorkspaceContext } from "./group-workspace";

const COLUMNS: Array<{ field: MemberSortBy; labelKey: string }> = [
  { field: "login_name", labelKey: "adminUsers.loginName" },
  { field: "display_name", labelKey: "adminUsers.displayName" },
  { field: "role", labelKey: "members.role" },
];

export function GroupMembersPage() {
  const { t } = useTranslation();
  const state = useGroupWorkspaceContext();

  return (
    <div className="grid min-w-0 gap-2" data-testid="group-members">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-base font-semibold">{t("groups.membersTitle")}</h2>
          <Input
            className="w-56"
            value={state.memberSearch}
            aria-label={t("members.searchUserPlaceholder")}
            placeholder={t("members.searchUserPlaceholder")}
            data-testid="members-search"
            onChange={(event) => state.setMemberSearch(event.target.value)}
          />
        </div>
        {state.canManage ? (
          <Button type="button" size="sm" data-testid="members-add" onClick={state.openCreateMember}>
            {t("members.add")}
          </Button>
        ) : null}
      </div>

      <div className="min-w-0 overflow-x-auto">
        <Table data-testid="members-table">
          <TableHeader>
            <TableRow>
              {COLUMNS.map((column) => (
                <TableHead key={column.field}>
                  <button
                    type="button"
                    className="inline-flex items-center gap-1"
                    onClick={() => state.setMemberSort(
                      column.field,
                      state.memberSort?.field === column.field && state.memberSort.direction === "asc" ? "desc" : "asc",
                    )}
                  >
                    {t(column.labelKey)}
                    {state.memberSort?.field === column.field ? (state.memberSort.direction === "asc" ? " ↑" : " ↓") : ""}
                  </button>
                </TableHead>
              ))}
              {state.canManage ? <TableHead>{t("common.actions")}</TableHead> : null}
            </TableRow>
          </TableHeader>
          <TableBody>
            {state.membersLoading && state.members.length === 0 ? (
              <TableRow>
                <TableCell colSpan={state.canManage ? 4 : 3} className="py-8 text-sm text-muted-foreground">
                  {t("common.loading")}
                </TableCell>
              </TableRow>
            ) : state.members.length === 0 ? (
              <TableRow>
                <TableCell colSpan={state.canManage ? 4 : 3} className="py-8 text-sm text-muted-foreground">
                  {t("members.empty")}
                </TableCell>
              </TableRow>
            ) : state.members.map((member) => (
              <TableRow key={member.login_name} data-testid="members-row">
                <TableCell className="font-mono text-xs">{member.login_name}</TableCell>
                <TableCell className="text-sm">{member.display_name}</TableCell>
                <TableCell>
                  <Badge variant={roleBadgeVariant(member.role)}>
                    {membershipRoleLabel(t, member.role)}
                  </Badge>
                </TableCell>
                {state.canManage ? (
                  <TableCell>
                    <div className="flex gap-2">
                      <Button type="button" size="sm" variant="outline" onClick={() => state.openEditMember(member)}>
                        {t("common.edit")}
                      </Button>
                      <Button type="button" size="sm" variant="destructive" onClick={() => state.requestRemoveMember(member)}>
                        {t("common.delete")}
                      </Button>
                    </div>
                  </TableCell>
                ) : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      {state.memberPage ? (
        <TablePagination
          pagination={state.memberPage}
          onPage={state.setMemberPage}
          onPageSize={state.setPageSize}
        />
      ) : null}
    </div>
  );
}
