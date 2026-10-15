import { QueryClientProvider } from "@tanstack/react-query";
import { lazy, Suspense, useState } from "react";
import { createQueryClient } from "@/engine";
import { TooltipProvider } from "@/ui";
import { ShellProvider } from "@/shell/context";
import { Workbench } from "@/shell/Workbench";

const catalogueAllowed = import.meta.env.DEV || import.meta.env.VITE_DISPATCHED_MOCK === "1";
const Catalogue = catalogueAllowed ? lazy(() => import("./dev/Catalogue")) : null;

export default function App() {
  const [client] = useState(createQueryClient);
  if (Catalogue && window.location.hash === "#catalogue") {
    return (
      <Suspense fallback={null}>
        <Catalogue />
      </Suspense>
    );
  }
  return (
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ShellProvider>
          <Workbench />
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>
  );
}
