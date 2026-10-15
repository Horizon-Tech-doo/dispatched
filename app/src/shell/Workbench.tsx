import { Shell } from "./Shell";
import { NamespaceTree } from "./tree/NamespaceTree";
import { Workspace } from "./tabs/Workspace";

/** Fills the shell's slots. Task 17 adds the drawer here. */
export function Workbench() {
  return <Shell tree={<NamespaceTree />} workspace={<Workspace />} />;
}
