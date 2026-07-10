import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";

export function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

export function useErrorToast() {
  const { t } = useI18n();
  const toast = useToast();

  return (error: unknown, fallback: string) => {
    toast.add({
      severity: "error",
      summary: t("common.error"),
      detail: errorMessage(error, fallback),
      life: 5000,
    });
  };
}
