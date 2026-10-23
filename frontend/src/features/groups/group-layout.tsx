import { ArrowLeft, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link, NavLink, Outlet, useParams } from "react-router";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { GroupEntityDialog, GroupMemberDialog, MoveGroupDialog } from "./group-dialogs";
import { visibilityLabel } from "@/lib/visibility-label";

import { membershipRoleLabel } from "./group-labels";
import { GroupWorkspaceProvider, useGroupWorkspaceContext } from "./group-workspace";

function GroupWorkspaceDialogs() {
  const { t } = useTranslation();
  const state = useGroupWorkspaceContext();

  return (
    <>
      <GroupEntityDialog
        open={state.editGroupOpen}
        title={t("groups.editGroup")}
        busy={state.groupBusy}
        submitLabel={t("common.save")}
        showKey={false}
        initialName={state.group?.name}
        initialVisibility={state.group?.visibility}
        onOpenChange={state.setEditGroupOpen}
        onSubmit={state.saveGroup}
      />
      <GroupEntityDialog
        open={state.childDialogOpen}
        title={state.editingChild ? t("groups.editChild") : t("groups.createChild")}
        busy={state.childBusy}
        submitLabel={state.editingChild ? t("common.save") : t("groups.createChild")}
        showKey={!state.editingChild}
        initialKey={state.editingChild?.group_key}
        initialName={state.editingChild?.name}
        initialVisibility={state.editingChild?.visibility}
        onOpenChange={state.setChildDialogOpen}
        onSubmit={state.saveChildGroup}
      />
      <GroupMemberDialog
        open={state.memberDialogOpen}
        busy={state.memberBusy}
        editing={state.editingMember}
        suggestions={state.memberSuggestions}
        canAssignOwner={state.canOwn}
        onOpenChange={state.setMemberDialogOpen}
        onSearch={state.searchMemberCandidates}
        onSubmit={state.saveMember}
      />
      <MoveGroupDialog
        open={state.moveDialogOpen}
        busy={state.moveBusy}
        moving={state.movingGroup}
        suggestions={state.moveSuggestions}
        onOpenChange={state.setMoveDialogOpen}
        onSearch={state.searchMoveTargets}
        onSubmit={state.submitMove}
      />
      <ConfirmDialog
        open={!!state.confirm}
        title={state.confirm?.title ?? ""}
        message={state.confirm?.message ?? ""}
        acceptLabel={state.confirm?.acceptLabel}
        onAccept={state.acceptConfirm}
        onCancel={state.cancelConfirm}
      />
    </>
  );
}

function GroupLayoutBody({ groupPath }: { groupPath: string }) {
  const { t } = useTranslation();
  const state = useGroupWorkspaceContext();
  const base = `/groups/${encodeURIComponent(groupPath)}`;
  const tabs = [
    { to: base, label: t("groups.overviewTitle"), end: true },
    { to: `${base}/members`, label: t("groups.membersTitle"), end: false },
    { to: `${base}/settings`, label: t("groups.settingsTitle"), end: false },
  ];

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col gap-2" data-testid="group-shell">
      <header className="flex min-w-0 flex-wrap items-center gap-2 border-b pb-2">
        <h2 className="sr-only">{t("nav.groups")}</h2>
        <Button asChild variant="ghost" size="sm">
          <Link to="/groups" data-testid="group-back-to-groups">
            <ArrowLeft aria-hidden="true" />
            {t("library.backToGroups")}
          </Link>
        </Button>
        <div className="flex min-w-0 items-center gap-2">
          <h1 className="truncate text-lg font-semibold">{state.group?.name ?? groupPath}</h1>
          {state.group?.current_role ? (
            <Badge variant="secondary">{membershipRoleLabel(t, state.group.current_role)}</Badge>
          ) : null}
          {state.group?.visibility ? (
            <Badge variant="outline">{visibilityLabel(t, state.group.visibility)}</Badge>
          ) : null}
        </div>
        {state.loading ? <Loader2 aria-hidden="true" className="size-4 animate-spin text-muted-foreground" /> : null}
        <nav aria-label={t("groups.overviewTitle")} className="ml-auto flex items-center gap-1">
          {tabs.map((tab) => (
            <NavLink
              key={tab.to}
              to={tab.to}
              end={tab.end}
              className={({ isActive }) => cn(
                "rounded-md px-2.5 py-1.5 text-sm",
                isActive ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:bg-muted/60",
              )}
            >
              {tab.label}
            </NavLink>
          ))}
        </nav>
      </header>

      {state.error ? (
        <div role="alert" className="rounded-md border border-destructive/40 p-3 text-sm">
          {state.error instanceof Error ? state.error.message : t("groups.loadFailed")}
        </div>
      ) : null}

      <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-testid="group-section-content">
        <Outlet />
      </div>
      <GroupWorkspaceDialogs />
    </div>
  );
}

export function GroupLayout() {
  const params = useParams();
  const groupPath = params.groupPath ?? "";

  return (
    <GroupWorkspaceProvider groupPath={groupPath} key={groupPath}>
      <GroupLayoutBody groupPath={groupPath} />
    </GroupWorkspaceProvider>
  );
}
