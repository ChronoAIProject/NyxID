import { Outlet } from "@tanstack/react-router";
import { Shield } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";

export function AuthLayout() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background p-4">
      <div className="mb-8 flex items-center gap-2">
        <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary">
          <Shield className="h-6 w-6 text-primary-foreground" />
        </div>
        <span className="text-2xl font-bold text-foreground">NyxID</span>
      </div>

      <Card className="w-full max-w-md border-border/50 bg-card/80 backdrop-blur-sm">
        <CardContent className="p-8">
          <Outlet />
        </CardContent>
      </Card>

      <p className="mt-8 text-center text-xs text-muted-foreground">
        Secure identity and access management by NyxID
      </p>
    </div>
  );
}
