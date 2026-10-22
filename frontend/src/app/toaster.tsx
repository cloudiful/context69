import { Toaster as Sonner } from "sonner";

import { useResolvedTheme } from "../hooks/use-preferences";

export function AppToaster() {
  const theme = useResolvedTheme();

  return <Sonner theme={theme} position="bottom-right" closeButton />;
}
