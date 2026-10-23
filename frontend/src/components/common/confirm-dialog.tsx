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

/**
 * Confirmation for an irreversible or destructive settings action. The caller
 * owns the open state, so the action itself stays an explicit click.
 */
export function ConfirmDialog({
  open,
  title,
  message,
  acceptLabel,
  onAccept,
  onCancel,
}: {
  open: boolean;
  title: string;
  message: string;
  acceptLabel?: string;
  onAccept: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();

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
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
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
