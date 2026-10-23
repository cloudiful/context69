import { useTranslation } from "react-i18next";
import { Link } from "react-router";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export function NotFoundPage() {
  const { t } = useTranslation();

  return (
    <div className="grid min-h-screen place-items-center px-4">
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>
            <h1 className="text-base font-semibold">{t("notFound.title")}</h1>
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3">
          <p className="text-sm text-muted-foreground">{t("notFound.message")}</p>
          <Button asChild className="w-fit">
            <Link to="/search">{t("notFound.action")}</Link>
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
