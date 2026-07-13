import { useOverlay } from "@nuxt/ui/composables";

import AppConfirmModal from "../components/AppConfirmModal.vue";

interface ConfirmOptions {
  header: string;
  message: string;
  acceptLabel?: string;
  rejectLabel?: string;
  accept?: () => void | Promise<void>;
  reject?: () => void | Promise<void>;
}

export function useAppConfirm() {
  const overlay = useOverlay();
  const modal = overlay.create(AppConfirmModal);

  return {
    require(options: ConfirmOptions) {
      void modal.open({
        title: options.header,
        message: options.message,
        acceptLabel: options.acceptLabel,
        rejectLabel: options.rejectLabel,
      }).then((accepted) => accepted ? options.accept?.() : options.reject?.());
    },
  };
}
