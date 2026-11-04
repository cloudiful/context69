import { useTranslation } from "react-i18next";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";

/** The explicit consent a duplicate-risk confirmation requires before accepting. */
export interface ConfirmRiskAcknowledgement {
  label: string;
  description?: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}

/**
 * Confirmation for an irreversible or destructive settings action. The caller
 * owns the open state, so the action itself stays an explicit click. An
 * optional risk acknowledgement adds one explicit checkbox; the accept action
 * stays disabled until that checkbox is checked.
 */
export function ConfirmDialog({
  open,
  title,
  message,
  acceptLabel,
  onAccept,
  onCancel,
  riskAcknowledgement,
}: {
  open: boolean;
  title: string;
  message: string;
  acceptLabel?: string;
  onAccept: () => void;
  onCancel: () => void;
  riskAcknowledgement?: ConfirmRiskAcknowledgement;
}) {
  const { t } = useTranslation();
  const acceptDisabled = riskAcknowledgement !== undefined && !riskAcknowledgement.checked;

  return (
    <AlertDialog open={open} onOpenChange={(next) => {
      if (!next) {
        onCancel();
      }
    }}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{message}</AlertDialogDescription>
        </AlertDialogHeader>
        {riskAcknowledgement
          ? (
              <div className="flex items-start gap-2 rounded-md border p-2">
                <Checkbox
                  id="confirm-risk-acknowledgement"
                  checked={riskAcknowledgement.checked}
                  onCheckedChange={(checked) => riskAcknowledgement.onCheckedChange(checked === true)}
                />
                <div className="grid gap-1">
                  <Label htmlFor="confirm-risk-acknowledgement">{riskAcknowledgement.label}</Label>
                  {riskAcknowledgement.description
                    ? (
                        <p className="text-sm text-muted-foreground">
                          {riskAcknowledgement.description}
                        </p>
                      )
                    : null}
                </div>
              </div>
            )
          : null}
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={acceptDisabled}
            onClick={(event) => {
              event.preventDefault();
              onAccept();
            }}
          >
            {acceptLabel ?? t("common.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
