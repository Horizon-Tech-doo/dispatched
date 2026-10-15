import { Drawer } from "./drawer/Drawer";
import { Shell } from "./Shell";
import { NamespaceTree } from "./tree/NamespaceTree";
import { Workspace } from "./tabs/Workspace";

/** Fills the shell's slots. */
export function Workbench() {
  return <Shell tree={<NamespaceTree />} workspace={<Workspace />} drawer={<Drawer />} />;
}
