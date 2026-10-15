import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport, type Scenario } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { ShellProvider } from "./context";
import { Workbench } from "./Workbench";

export function renderShell(scenario: Scenario = "default", seed: Record<string, unknown> = {}) {
  const mock = createMockTransport(scenario, 0, seed);
  setTransport(mock);
  const client = createQueryClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ShellProvider>
          <Workbench />
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { ...utils, mock, client };
}
