import { act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect } from "react";
import { useShell, type SheetState } from "../context";

/** Opens `sheet` once the shell is connected (tests). */
export function OpenSheet({ sheet }: { sheet: Exclude<SheetState, null> }) {
  const { live, openSheet } = useShell();
  useEffect(() => {
    if (live) openSheet(sheet);
    // Once, when the shell comes up.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [live]);
  return null;
}

/** The arguments of the last call of `cmd`. */
export const lastCall = <T,>(mock: { calls: { cmd: string; args?: unknown }[] }, cmd: string) => mock.calls.filter((c) => c.cmd === cmd).at(-1)?.args as T | undefined;

/** Types into a field inside the workbench: jsdom's panels swallow the pointer-down a click would focus it with. */
export async function typeInto(el: HTMLElement, text: string) {
  await act(async () => el.focus());
  await userEvent.keyboard(text);
}
