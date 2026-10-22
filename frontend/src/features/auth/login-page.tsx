import { Loader2 } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearchParams } from "react-router";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAuthSession } from "@/hooks/use-auth";
import { AuthError, login } from "@/lib/auth-store";

const DEFAULT_REDIRECT = "/search";

export function resolveLoginRedirect(redirect: string | null): string {
  if (!redirect || !redirect.startsWith("/") || redirect.startsWith("//")) {
    return DEFAULT_REDIRECT;
  }

  return redirect;
}

export function LoginPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { user } = useAuthSession();
  const [loginName, setLoginName] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<{ loginName?: string; password?: string }>({});

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (busy) {
      return;
    }

    const nextErrors: { loginName?: string; password?: string } = {};
    if (!loginName.trim()) {
      nextErrors.loginName = t("auth.validation.loginNameRequired");
    }
    if (!password) {
      nextErrors.password = t("auth.validation.passwordRequired");
    }

    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) {
      return;
    }

    setBusy(true);
    try {
      await login({ login_name: loginName.trim(), password });
      navigate(resolveLoginRedirect(searchParams.get("redirect")), { replace: true });
    } catch (error) {
      const reason = error instanceof AuthError ? error.reason : "unknown";
      const message = reason === "invalid_credentials"
        ? t("auth.invalidCredentials")
        : reason === "network"
          ? t("auth.networkError")
          : t("auth.loginFailed");
      toast.error(t("auth.loginFailedTitle"), { description: message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="grid min-h-screen w-full place-items-center px-3 py-6">
      <Card className="w-full max-w-[24rem]">
        <CardHeader>
          <CardTitle>
            <h1 className="text-base font-semibold">{t("auth.title")}</h1>
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          {user ? (
            <div className="grid gap-1 rounded-md border px-3 py-2">
              <span className="text-sm font-semibold">{user.display_name}</span>
              <span className="text-xs text-muted-foreground">{user.login_name}</span>
            </div>
          ) : null}

          {searchParams.get("reason") === "expired" ? (
            <Alert>
              <AlertTitle>{t("auth.sessionUnavailable")}</AlertTitle>
              <AlertDescription>{t("auth.sessionUnavailableMessage")}</AlertDescription>
            </Alert>
          ) : null}

          <form className="grid gap-3" onSubmit={submit} noValidate>
            <div className="grid gap-1.5">
              <Label htmlFor="login-name">{t("auth.loginName")}</Label>
              <Input
                id="login-name"
                name="login_name"
                value={loginName}
                onChange={(event) => setLoginName(event.target.value)}
                placeholder={t("auth.loginNamePlaceholder")}
                autoComplete="username"
                disabled={busy}
                aria-invalid={errors.loginName ? true : undefined}
                aria-describedby={errors.loginName ? "login-name-error" : undefined}
              />
              {errors.loginName ? (
                <p id="login-name-error" className="text-xs text-destructive">{errors.loginName}</p>
              ) : null}
            </div>

            <div className="grid gap-1.5">
              <Label htmlFor="login-password">{t("auth.password")}</Label>
              <Input
                id="login-password"
                name="password"
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder={t("auth.passwordPlaceholder")}
                autoComplete="current-password"
                disabled={busy}
                aria-invalid={errors.password ? true : undefined}
                aria-describedby={errors.password ? "login-password-error" : undefined}
              />
              {errors.password ? (
                <p id="login-password-error" className="text-xs text-destructive">{errors.password}</p>
              ) : null}
            </div>

            <Button type="submit" className="w-full" disabled={busy} aria-busy={busy}>
              {busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
              {busy ? t("auth.signingIn") : t("auth.signIn")}
            </Button>
          </form>
        </CardContent>
      </Card>
    </main>
  );
}
