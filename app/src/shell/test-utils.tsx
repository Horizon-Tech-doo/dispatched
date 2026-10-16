import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport, type Scenario } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { ShellProvider } from "./context";
import { activityStore, jobStore } from "./drawer/activity";
import { resetViewStates } from "./messages/viewStore";
import { installStore } from "./sheets/installState";
import { forgetCounts } from "./tabs/subtabs";
import { Workbench } from "./Workbench";

/** `extra` renders beside the workbench inside the providers (a probe for shell state). */
export function renderShell(scenario: Scenario = "default", seed: Record<string, unknown> = {}, extra?: ReactNode) {
  activityStore.clear();
  jobStore.clear();
  installStore.reset();
  resetViewStates();
  forgetCounts();
  const mock = createMockTransport(scenario, 0, seed);
  setTransport(mock);
  const client = createQueryClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ShellProvider>
          <Workbench />
          {extra}
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { ...utils, mock, client };
}
