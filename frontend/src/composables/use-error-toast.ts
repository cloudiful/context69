import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";

export function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

export function useErrorToast() {
  const { t } = useI18n();
  const toast = useToast();

  return (error: unknown, fallback: string) => {
    toast.add({
      color: "error",
      title: t("common.error"),
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  };
}
